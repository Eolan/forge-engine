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
    float4 data = slot.fresh ? float4(0.0, 0.0, 0.0, PROBE_INACTIVE) : field[0].probe_data[probe];
    float state_before = probe_state(data.w);
    float age = probe_age(data.w);
    if (age >= PROBE_SETTLE_UPDATES)
    {
        // Settled: it keeps its place and state; a reset lasts one update.
        if (state_before == PROBE_RESET)
            field[0].probe_data[probe] = float4(data.xyz, PROBE_ACTIVE + 4.0 * age);
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
    float limit = 0.45 * s;
    float length_now = length(offset);
    if (length_now > limit)
        offset *= limit / length_now;
    float state = !inside && near ? PROBE_ACTIVE : PROBE_INACTIVE;
    if (state == PROBE_ACTIVE && state_before == PROBE_INACTIVE)
        state = PROBE_RESET;
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
static const uint BLEND_GROUP = DISTANCE_TILE * DISTANCE_TILE; // 256 threads
static const float DISTANCE_SHARPNESS = 50.0;

groupshared float3 s_dir[PROBE_MAX_RAYS];
groupshared float4 s_ray[PROBE_MAX_RAYS];
groupshared float3 s_irradiance[IRRADIANCE_TEXELS * IRRADIANCE_TEXELS];
groupshared float2 s_distance[DISTANCE_TEXELS * DISTANCE_TEXELS];

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
    if (state == PROBE_INACTIVE)
        return; // the whole group: its maps keep what they held
    ProbeSlot slot = probe_slot(field, probe);
    bool reset = slot.fresh || state == PROBE_RESET;
    float h = reset ? 0.0 : field[0].hysteresis;
    uint random = field[0].rays - PROBE_FIXED_RAYS;
    uint first = probe * field[0].rays + PROBE_FIXED_RAYS;
    for (uint i = thread; i < random; i += BLEND_GROUP)
    {
        s_dir[i] = probe_ray_direction(field, PROBE_FIXED_RAYS + i);
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
        float3 measured = total > 1e-3 ? sum / total : (reset ? float3(0.0) : old);
        // `select`, not `lerp`: a map that was never written may hold anything, NaN included.
        s_irradiance[thread] = h > 0.0 ? lerp(measured, old, h) : measured;
    }
    if (thread < DISTANCE_TEXELS * DISTANCE_TEXELS)
    {
        uint2 t = uint2(thread % DISTANCE_TEXELS, thread / DISTANCE_TEXELS);
        float3 dir = texel_direction(t, DISTANCE_TEXELS);
        float farthest = 1.5 * sqrt(3.0) * slot.spacing; // a miss counts as this far
        float2 sum = float2(0.0);
        float total = 0.0;
        for (uint i = 0; i < random; i++)
        {
            float w = pow(max(0.0, dot(dir, s_dir[i])), DISTANCE_SHARPNESS);
            float d = min(abs(s_ray[i].w), farthest);
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
        irradiance[irradiance_tile + u] = float4(s_irradiance[e.y * IRRADIANCE_TEXELS + e.x], 1.0);
    }
}
