// The probes' update after their rays (issue #53, probes.slang): relocation and
// classification from the fixed rays, then the blend of the other rays into the octahedral
// maps, with their borders.
import bindless;
import probes;

struct Push
{
    ProbeField* field;
    uint probe_count; // probes of every cascade
    uint pad;
};

[[vk::push_constant]]
Push push;

// ------------------------------------------------------------ probe state ---

// One thread per probe, from its 32 fixed rays (the 2021 paper's relocation and
// classification, simplified):
// - more than a quarter of back faces: the probe is inside something. It moves through the
//   nearest back face, and stays inactive until its rays see the outside.
// - a surface nearer than a quarter of a cell: it moves away from it by the difference.
// - active when the outside of some surface lies within one cell (the eight cells around it
//   are where the points it lights are), else inactive: nothing there needs it.
// The fixed rays reach 3.5 cells (PROBE_FIXED_REACH in meshlet.slang); a probe deep inside a
// building sees no outside within one cell and turns inactive without counting its walls.
// The offset stays within 0.45 of a cell, so probes never swap places. A new probe starts at
// its cell's centre, and settles after PROBE_SETTLE_UPDATES (probes.slang).
[shader("compute")]
[numthreads(64, 1, 1)]
void probe_state_main(uint3 id : SV_DispatchThreadID)
{
    uint probe = id.x;
    if (probe >= push.probe_count)
        return;
    ProbeField* field = push.field;
    ProbeSlot slot = probe_slot(field, probe);
    if (slot.unused)
        return;
    float4 data = slot.fresh ? float4(0.0, 0.0, 0.0, PROBE_INACTIVE) : field[0].probe_data[probe];
    float state_before = probe_state(data.w);
    float age = probe_age(data.w);
    // D-078: a cascade's probe in a room whose own grid lights it whole retires (it would light
    // nothing the grid does not); once the grid fades out it starts over, young and measured anew.
    bool retire = false;
    if (slot.grid == ROOM_NONE && field[0].room_count > 0u && !slot.fresh)
    {
        uint region = field[0].probe_region[probe];
        uint grid = region == REGION_NONE ? ROOM_NONE : room_grid_of(field, region, slot.position + data.xyz);
        retire = grid != ROOM_NONE && field[0].room_grids[grid].fade >= 1.0;
    }
    if (state_before == PROBE_RETIRED && !retire)
    {
        field[0].probe_data[probe] = float4(data.xyz, PROBE_RESET);
        return;
    }
    if (age >= PROBE_SETTLE_UPDATES)
    {
        if (retire)
        {
            if (state_before == PROBE_ACTIVE || state_before == PROBE_RESET)
                field[0].probe_data[probe] = float4(data.xyz, PROBE_RETIRED + 4.0 * (PROBE_SETTLE_UPDATES + 1.0));
            return;
        }
        // Settled: it keeps its place and state; a reset lasts one update. The first time,
        // its age becomes PROBE_SETTLE_UPDATES + 1, which tells the blend it settled before
        // this update (`probe_due`, issue #103).
        if (state_before == PROBE_RESET || age == PROBE_SETTLE_UPDATES)
        {
            float state = state_before == PROBE_RESET ? PROBE_ACTIVE : state_before;
            field[0].probe_data[probe] = float4(data.xyz, state + 4.0 * (PROBE_SETTLE_UPDATES + 1.0));
        }
        return;
    }
    float s = slot.spacing;
    uint back = 0;
    float closest_back = PROBE_MISS_DISTANCE;
    float3 back_dir = float3(0.0);
    float closest_front = PROBE_MISS_DISTANCE;
    float3 front_dir = float3(0.0);
    bool near = false;
    uint first = probe * field[0].rays;
    for (uint i = 0; i < PROBE_FIXED_RAYS; i++)
    {
        float t = unpack_ray(field[0].ray_data[first + i]).w;
        float3 dir = fibonacci_direction(i, PROBE_FIXED_RAYS);
        float reach = abs(t) * max(abs(dir.x), max(abs(dir.y), abs(dir.z))); // the hit's distance in cells, times s
        if (t < 0.0)
        {
            back++;
            if (-t < closest_back)
            {
                closest_back = -t;
                back_dir = dir;
            }
        }
        else if (t < closest_front)
        {
            closest_front = t;
            front_dir = dir;
        }
        if (t >= 0.0 && reach <= s)
            near = true;
    }
    bool inside = back * 4 > PROBE_FIXED_RAYS;
    float3 offset = data.xyz;
    float keep_off = 0.25 * s;
    if (inside)
        offset += back_dir * (closest_back + 0.5 * keep_off);
    else if (closest_front < keep_off)
        offset -= front_dir * (keep_off - closest_front);
    // A room grid's probe (D-078) stays on its lattice: moved out of a solid standing in its room
    // (a landing's guard, a wardrobe), it went out on the nearer side, and from the stairwell's it
    // lit the landing's floor along the guard's foot in spots a metre apart (the owner,
    // 2026-10-24: "light spots at the wall/floor junction"). One inside something is inactive,
    // and its neighbours light round it.
    if (slot.grid != ROOM_NONE)
        offset = float3(0.0);
    float limit = 0.45 * s;
    float length_now = length(offset);
    if (length_now > limit)
        offset *= limit / length_now;
    // A room grid's probe (D-078) lights its room wherever it stands in it, near a surface or not:
    // its grid's lookup has no other probes to fall back on.
    float state = !inside && (near || slot.grid != ROOM_NONE) ? PROBE_ACTIVE : PROBE_INACTIVE;
    // Its region (D-068), where it now stands; kept once it settles. In a window's recess or a
    // doorway it has none: pushed into a reveal, half in the sun, it lit the window's head.
    if (field[0].region_count > 0u)
    {
        RegionHit hit = region_at(field, slot.position + offset);
        uint region = hit.kind == REGION_ROOM ? hit.region : REGION_NONE;
        if (slot.grid != ROOM_NONE)
        {
            // A room grid's probe (D-078) is its room's. One standing outside it (in a wall, or
            // past an outline that is not a rectangle) lights nothing of it.
            uint own = field[0].room_grids[slot.grid].region;
            if (hit.region != own && hit.other != own)
                state = PROBE_INACTIVE;
            region = own;
        }
        field[0].probe_region[probe] = region;
    }
    if (state == PROBE_ACTIVE && state_before == PROBE_INACTIVE)
        state = PROBE_RESET;
    if (retire && state != PROBE_INACTIVE)
        state = PROBE_RETIRED;
    field[0].probe_data[probe] = float4(offset, state + 4.0 * (age + 1.0));
}

// ------------------------------------------------------------------- blend ---

// One workgroup per probe: its rays into groupshared memory, then one thread per interior
// texel of each map sums the rays around its direction and blends the sum into what the map
// held (the hysteresis), and every texel of the tiles, borders included, is written.
// Irradiance: the rays' radiance weighted by the cosine to the texel's direction, divided by
// the weights (back faces left out: their light is inside something). Distance: the rays'
// distances and their squares weighted by the cosine to the power 50, a narrow lobe, so the
// map keeps the shape of what surrounds the probe.
//
// In a room-sized cascade (PROBE_ROOM_SPACING) the lobe is the cosine to the power 10 (#283).
// There probes stand a few tens of centimetres from ceilings, reveals and a stair's opening, and
// see them at grazing angles. Seen so, an edge's distances change from one texel to the next by
// more than the bilinear lookup can follow: the test's boundary took the map's 14-texel grid,
// and a sawtooth shadow lay on the village hall's ceiling and along a window's recess. The wider
// lobe smooths the map across its texels, so the boundary is a soft gradient. Coarser cascades
// keep 50: their light stays the same to the bit.
static const uint BLEND_GROUP = DISTANCE_TILE * DISTANCE_TILE; // 256 threads
static const float DISTANCE_SHARPNESS = 50.0;
static const float DISTANCE_SHARPNESS_ROOM = 10.0;
// The share of its maps a young probe's update keeps: after its eight young updates, its first
// keeps 0.5^8, 0.4 %.
static const float YOUNG_HYSTERESIS = 0.25;
// Adaptive hysteresis (#336, as DDGI lowers its hysteresis on a large change): a settled probe
// whose measured light differs from its maps' by more than CHANGE_START of the brighter (the
// means of its irradiance texels' luminance) keeps less of its maps, down to CHANGED_HYSTERESIS
// from CHANGE_FULL on. A door opened or a lamp switched settles in a few updates, where 0.97
// took 1.7 s (and a doorway's chain of probes, each waiting on the next, about 4 s); the noise
// of a settled probe's mean over its rays stays far under the start.
static const float CHANGE_START = 0.3;
static const float CHANGE_FULL = 0.8;
static const float CHANGED_HYSTERESIS = 0.5;
// How many standard errors of an update's mean a change must pass before it counts (a game's
// forest: the rotations' sampling alone moved the means by more than CHANGE_START).
static const float CHANGE_NOISE = 2.0;
// A change that lasts passes anyway (a game's doorway: opened, its room's probes saw a few bright
// rays among dark ones, as under a crown, and the noise test held its light back, 2 s to settle
// became 5 s). The rays' rotations repeat every 8 frames with the jitter, so over a probe's last
// DRIFT_UPDATES updates the rotations' noise sums to about nothing, while a door's light keeps
// its sign: the mean of those updates' gaps, each a share of the brighter as the change is, counts
// as a change too, from DRIFT_START to DRIFT_FULL: rid of the noise, it can start lower (with the
// 30 % start the door's light came in a step and then tailed off over 4 s, where single updates
// had crossed it on their noise). It counts only as far as it stands clear of the gaps' own
// spread, from DRIFT_SURE_START to DRIFT_SURE_FULL standard errors (a t-test over the 8): where
// the rotations don't cancel (a game's camera inside a dense crown, its gaps large and of either
// sign) the 10 % start alone let the flicker back, three times the noise test's, while a door's
// gaps share their sign. They ride in the alpha of the probe's first irradiance texels, which
// nothing samples.
static const uint DRIFT_UPDATES = 8;
static const float DRIFT_START = 0.1;
static const float DRIFT_FULL = 0.4;
static const float DRIFT_SURE_START = 2.5;
static const float DRIFT_SURE_FULL = 4.0;

groupshared float3 s_dir[PROBE_MAX_RAYS];
groupshared float4 s_ray[PROBE_MAX_RAYS];
groupshared float3 s_irradiance[IRRADIANCE_TEXELS * IRRADIANCE_TEXELS];
groupshared float3 s_irradiance_old[IRRADIANCE_TEXELS * IRRADIANCE_TEXELS];
groupshared float2 s_distance[DISTANCE_TEXELS * DISTANCE_TEXELS];
groupshared float s_hysteresis;
groupshared float s_drift[DRIFT_UPDATES];

[shader("compute")]
[numthreads(BLEND_GROUP, 1, 1)]
void probe_blend_main(uint3 group : SV_GroupID, uint thread : SV_GroupIndex)
{
    uint probe = group.y * 65535u + group.x;
    if (probe >= push.probe_count)
        return;
    ProbeField* field = push.field;
    float4 data = field[0].probe_data[probe];
    float state = probe_state(data.w);
    if (state == PROBE_INACTIVE || state == PROBE_RETIRED)
        return; // the whole group: its maps keep what they held
    ProbeSlot slot = probe_slot(field, probe);
    if (slot.unused)
        return; // a room grid's slot given back (D-078): nothing samples it
    float age = probe_age(data.w);
    if (!probe_due(field, probe, slot, age, true))
        return; // not its turn: its rays were not traced (issue #103)
    bool reset = slot.fresh || state == PROBE_RESET;
    // A settled probe's update comes every `cadence` frames and keeps hysteresis^cadence, so
    // a change settles in the same time. A young one keeps less (#171): its first updates
    // measured surfaces whose own probes were not measured yet and lent them the open sky's
    // light, too bright indoors, and at the settled hysteresis that start took 200 frames to
    // wear off.
    float h = reset ? 0.0 : (age > PROBE_SETTLE_UPDATES ? field[0].settled_hysteresis : min(field[0].hysteresis, YOUNG_HYSTERESIS));
    uint rays = probe_rays(field, slot);
    uint random = rays - PROBE_FIXED_RAYS;
    uint first = probe * field[0].rays + PROBE_FIXED_RAYS;
    for (uint i = thread; i < random; i += BLEND_GROUP)
    {
        s_dir[i] = probe_ray_direction(field, PROBE_FIXED_RAYS + i, rays);
        s_ray[i] = unpack_ray(field[0].ray_data[first + i]);
    }
    GroupMemoryBarrierWithGroupSync();

    uint2 irradiance_tile = probe_tile(field, probe, IRRADIANCE_TILE);
    uint2 distance_tile = probe_tile(field, probe, DISTANCE_TILE);
    RWTexture2D<float4> irradiance = g_storage_images_rgba16f[field[0].irradiance_storage];
    RWTexture2D<float2> distance = g_storage_images_rg16f[field[0].distance_storage];
    if (thread < IRRADIANCE_TEXELS * IRRADIANCE_TEXELS)
    {
        uint2 t = uint2(thread % IRRADIANCE_TEXELS, thread / IRRADIANCE_TEXELS);
        float3 dir = texel_direction(t, IRRADIANCE_TEXELS);
        float3 sum = float3(0.0);
        float total = 0.0;
        for (uint i = 0; i < random; i++)
        {
            float4 ray = s_ray[i];
            if (ray.w < 0.0)
                continue;
            float w = max(0.0, dot(dir, s_dir[i]));
            sum += w * ray.rgb;
            total += w;
        }
        float3 old = irradiance[irradiance_tile + 1 + t].rgb;
        s_irradiance[thread] = total > 1e-3 ? sum / total : (reset ? float3(0.0) : old);
        s_irradiance_old[thread] = old;
    }
    GroupMemoryBarrierWithGroupSync();
    if (thread == 0)
    {
        float kept = h;
        for (uint k = 0; k < DRIFT_UPDATES; k++)
            s_drift[k] = 0.0;
        if (!reset && age > PROBE_SETTLE_UPDATES)
        {
            float3 luma = float3(0.2126, 0.7152, 0.0722);
            float now = 0.0;
            float before = 0.0;
            for (uint i = 0; i < IRRADIANCE_TEXELS * IRRADIANCE_TEXELS; i++)
            {
                now += dot(s_irradiance[i], luma);
                before += dot(s_irradiance_old[i], luma);
            }
            // What this update's own sampling explains: the rays' luminance spread over √n, a
            // texel taking about a quarter of them, counted CHANGE_NOISE times. Under a crown each
            // of the rays' rotations meets the sky through other gaps, and their means parted by
            // more than CHANGE_START with nothing changed: the probes followed the 8-frame cycle of
            // rotations and the foliage flickered with the camera still (a game's forest).
            float sum = 0.0;
            float squares = 0.0;
            float n = 0.0;
            for (uint i = 0; i < random; i++)
            {
                if (s_ray[i].w < 0.0)
                    continue;
                float l = dot(s_ray[i].rgb, luma);
                sum += l;
                squares += l * l;
                n += 1.0;
            }
            float mean = sum / max(n, 1.0);
            float spread = sqrt(max(squares / max(n, 1.0) - mean * mean, 0.0));
            float texels = float(IRRADIANCE_TEXELS * IRRADIANCE_TEXELS);
            float noise = CHANGE_NOISE * 2.0 * spread / sqrt(max(n, 1.0));
            float brighter = max(max(now, before) / texels, 1e-12);
            float gap = (now - before) / texels / brighter;
            float change = max(abs(now - before) / texels - noise, 0.0) / brighter;
            // The last updates' gaps, this one first, their mean and how sure it is.
            s_drift[0] = gap;
            float lasting = gap;
            float lasting_squares = gap * gap;
            for (uint k = 1; k < DRIFT_UPDATES; k++)
            {
                float g = irradiance[irradiance_tile + 1 + uint2((k - 1) % IRRADIANCE_TEXELS, (k - 1) / IRRADIANCE_TEXELS)].a;
                g = abs(g) <= 1.0 ? g : 0.0; // a map never written holds anything, NaN included
                s_drift[k] = g;
                lasting += g;
                lasting_squares += g * g;
            }
            float updates = float(DRIFT_UPDATES);
            float drift_mean = lasting / updates;
            float spread_now = max(lasting_squares / updates - drift_mean * drift_mean, 0.0);
            float deviation = sqrt(spread_now * updates / (updates - 1.0));
            float sure = abs(drift_mean) / max(deviation / sqrt(updates), 1e-6);
            float lowered = max(smoothstep(CHANGE_START, CHANGE_FULL, change),
                                smoothstep(DRIFT_START, DRIFT_FULL, abs(drift_mean))
                                    * smoothstep(DRIFT_SURE_START, DRIFT_SURE_FULL, sure));
            kept = min(h, lerp(h, CHANGED_HYSTERESIS, lowered));
        }
        s_hysteresis = kept;
    }
    GroupMemoryBarrierWithGroupSync();
    h = s_hysteresis;
    if (thread < IRRADIANCE_TEXELS * IRRADIANCE_TEXELS)
    {
        // `select`, not `lerp`: a map that was never written may hold anything, NaN included.
        float3 measured = s_irradiance[thread];
        s_irradiance[thread] = h > 0.0 ? lerp(measured, s_irradiance_old[thread], h) : measured;
    }
    if (thread < DISTANCE_TEXELS * DISTANCE_TEXELS)
    {
        uint2 t = uint2(thread % DISTANCE_TEXELS, thread / DISTANCE_TEXELS);
        float3 dir = texel_direction(t, DISTANCE_TEXELS);
        float farthest = 1.5 * sqrt(3.0) * slot.spacing; // a miss counts as this far
        float sharpness = slot.spacing <= PROBE_ROOM_SPACING ? DISTANCE_SHARPNESS_ROOM : DISTANCE_SHARPNESS;
        float2 sum = float2(0.0);
        float total = 0.0;
        for (uint i = 0; i < random; i++)
        {
            float w = pow(max(0.0, dot(dir, s_dir[i])), sharpness);
            // A back face's hit at a fifth of its distance (the 2021 paper's "shortened by 80%",
            // RTXGI's): what the probe meets the back of is the inside of something, and points
            // past it fail the visibility test sooner (D-068). Not to 0, which would fail every
            // point that way in a probe that sees a few back faces. The relocation (above) keeps
            // the full distance.
            float t = s_ray[i].w;
            float d = min(t < 0.0 ? -0.2 * t : t, farthest);
            sum += w * float2(d, d * d);
            total += w;
        }
        float2 old = distance[distance_tile + 1 + t];
        float2 measured = total > 1e-6 ? sum / total : (reset ? float2(farthest, farthest * farthest) : old);
        s_distance[thread] = h > 0.0 ? lerp(measured, old, h) : measured;
    }
    GroupMemoryBarrierWithGroupSync();

    uint2 t = uint2(thread % DISTANCE_TILE, thread / DISTANCE_TILE);
    uint2 d = border_source(t, DISTANCE_TEXELS);
    distance[distance_tile + t] = s_distance[d.y * DISTANCE_TEXELS + d.x];
    if (thread < IRRADIANCE_TILE * IRRADIANCE_TILE)
    {
        uint2 u = uint2(thread % IRRADIANCE_TILE, thread / IRRADIANCE_TILE);
        uint2 e = border_source(u, IRRADIANCE_TEXELS);
        uint i = e.y * IRRADIANCE_TEXELS + e.x;
        irradiance[irradiance_tile + u] = float4(s_irradiance[i], i < DRIFT_UPDATES ? s_drift[i] : 0.0);
    }
}

// ------------------------------------------------------------- probe wake ---
//
// Moving geometry (#79, issue #69): a settled probe keeps its place and its state, which the
// movers may have changed. A probe whose cell a mover entered or left this frame starts its
// settling over (its age back to 0: relocation and classification again for
// PROBE_SETTLE_UPDATES updates). A probe's cell counts as reached by a mover within a cell of
// it, where the points it lights are. A mover standing over a probe leaves it alone, so a
// parked one does not wake it every frame.

// Mirrors `Instance` in meshlet.slang (80 bytes): the fields the wake reads.
struct MoverRecord
{
    int3 cell;
    uint mesh;
    float3 local;
    float scale;
    float4 rotation;
    float3 center;
    float radius;
    uint id;
    uint material;
    uint flags;   // `Instance::flags`: MOVER_NO_PROBE_RAYS
    uint texture;
};

// `INSTANCE_NO_PROBE_RAYS` in meshlet.slang: a mover the probes' rays never meet.
static const uint MOVER_NO_PROBE_RAYS = 16u;

struct WakePush
{
    ProbeField* field;
    MoverRecord* now;    // the movers' records of this frame
    MoverRecord* before; // of the frame before
    uint count;
    uint cascades;
    int4 origin_cell;    // the scene frame's origin, which the probes work in
    float4 origin_local;
};

[[vk::push_constant]]
WakePush wake_push;

// `CELL_SIZE` in meshlet.slang: metres of an instance's cell.
static const float MOVER_CELL_SIZE = 1024.0;
// Cells a mover's reach covers along each axis at most: the rest of a large mover waits.
static const int WAKE_REACH = 4;

// A mover's bounding sphere in the scene frame: its centre, its radius.
float4 mover_sphere(MoverRecord r)
{
    float3 c = float3(r.cell - wake_push.origin_cell.xyz) * MOVER_CELL_SIZE + (r.center - wake_push.origin_local.xyz);
    return float4(c, r.radius);
}

// Whether a sphere comes within `reach` of the box `lo..hi`.
bool sphere_reaches(float4 s, float3 lo, float3 hi, float reach)
{
    float3 d = max(max(lo - s.xyz, s.xyz - hi), 0.0);
    return dot(d, d) <= (s.w + reach) * (s.w + reach);
}

// A thread per mover and cascade.
[shader("compute")]
[numthreads(64, 1, 1)]
void probe_wake_main(uint3 id : SV_DispatchThreadID)
{
    WakePush p = wake_push;
    if (id.x >= p.count * p.cascades)
        return;
    uint mover = id.x / p.cascades;
    uint c = id.x % p.cascades;
    ProbeField* field = p.field;
    float4 now = mover_sphere(p.now[mover]);
    float4 before = mover_sphere(p.before[mover]);
    // A hidden mover (a scale of 0: one not given yet, a pool's free one) wakes nothing, nor one
    // shown or hidden: a pool's mover stands for the static instance it is promoted from or
    // demoted to, which the probes' rays meet all along (#338).
    if (now.w <= 0.0 || before.w <= 0.0)
        return;
    // Nor one the probes' rays never meet, here or before: it changes nothing they see, and its
    // wake would only make them settle again (#351: a game's grass ring, its patches moved round
    // the camera).
    if ((p.now[mover].flags & p.before[mover].flags & MOVER_NO_PROBE_RAYS) != 0u)
        return;
    ProbeCascade cascade = field[0].cascades[c];
    float s = cascade.spacing;
    int3 n = int3(cascade.counts);
    float3 lo = min(now.xyz - now.w, before.xyz - before.w) - s;
    float3 hi = max(now.xyz + now.w, before.xyz + before.w) + s;
    int3 first = max(int3(floor(lo / s)), cascade.origin);
    int3 last = min(min(int3(floor(hi / s)), cascade.origin + n - 1), first + WAKE_REACH - 1);
    for (int z = first.z; z <= last.z; z++)
        for (int y = first.y; y <= last.y; y++)
            for (int x = first.x; x <= last.x; x++)
            {
                int3 cell = int3(x, y, z);
                float3 box_lo = float3(cell) * s;
                float3 box_hi = box_lo + s;
                if (sphere_reaches(now, box_lo, box_hi, s) == sphere_reaches(before, box_lo, box_hi, s))
                    continue;
                uint probe = probe_index(field, c, cell);
                float w = field[0].probe_data[probe].w;
                if (probe_age(w) > PROBE_SETTLE_UPDATES)
                    field[0].probe_data[probe].w = probe_state(w);
            }
}

// ----------------------------------------------------------- regions' wake (#338) ---

// The probes about a set of region volumes added or removed (a streamed cell's, regions.rs):
// settled ones in its box, padded by their spacing, made young again, so that they take their
// region anew (`probe_state_main`) rather than every probe starting over.
struct WakeRegionsPush
{
    ProbeField* field;
    float4* boxes; // per box: its lowest corner (xyz), then its highest, scene frame
    uint count;    // boxes
    uint probes;   // probes of every cascade
};

[[vk::push_constant]]
WakeRegionsPush wake_regions_push;

// A thread per probe.
[shader("compute")]
[numthreads(64, 1, 1)]
void probe_wake_regions_main(uint3 id : SV_DispatchThreadID)
{
    WakeRegionsPush p = wake_regions_push;
    if (id.x >= p.probes)
        return;
    ProbeField* field = p.field;
    ProbeSlot slot = probe_slot(field, id.x);
    if (slot.fresh || slot.unused)
        return;
    for (uint b = 0u; b < p.count; b++)
    {
        float3 lo = p.boxes[2u * b].xyz - slot.spacing;
        float3 hi = p.boxes[2u * b + 1u].xyz + slot.spacing;
        if (any(slot.position < lo) || any(slot.position > hi))
            continue;
        float w = field[0].probe_data[id.x].w;
        if (probe_age(w) > PROBE_SETTLE_UPDATES)
            field[0].probe_data[id.x].w = probe_state(w);
        return;
    }
}
