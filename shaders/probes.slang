// Diffuse light from probes updated by ray queries (issue #53, D-008's T1 tier): Majercik et
// al., "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields" (JCGT 2019),
// with the relocation, classification and bias of Majercik et al., "Scaling Probe-Based
// Real-Time Dynamic Global Illumination for Production" (JCGT 2021). Written from the papers;
// NVIDIA's RTXGI-DDGI SDK was read for the constants, none of its code is used.
//
// Cascades of probes follow the camera, each twice as coarse as the one before. A probe
// holds, over the sphere of directions (octahedral maps), the irradiance its rays measured and
// the mean and mean square of the distance to the nearest surface. Shading blends the eight
// probes around a point, weighted by whether each can see it (a Chebyshev test on the
// distances), so light does not leak through walls. The shared data and the lookup live here;
// the passes that trace (`probe_trace_main`, meshlet.slang) and update (probe_update.slang)
// the probes, and the resolve, import them.
module probes;

import bindless;

public static const uint PROBE_MAX_CASCADES = 6;
// The first rays of every probe keep fixed directions (not rotated per frame), so relocation
// and classification see the same scene every frame; blending uses the others.
public static const uint PROBE_FIXED_RAYS = 32;
public static const uint PROBE_MAX_RAYS = 256;
// Texels a side of a probe's octahedral maps, then with their one-texel border.
public static const uint IRRADIANCE_TEXELS = 6;
public static const uint IRRADIANCE_TILE = IRRADIANCE_TEXELS + 2;
public static const uint DISTANCE_TEXELS = 14;
public static const uint DISTANCE_TILE = DISTANCE_TEXELS + 2;

// `ProbeField::probe_data` w holds a probe's state and its age, as state + 4 × age.
// The state says whether it lights anything. A probe inside geometry or with no surface
// within a cell of it is inactive: not blended, not sampled. One that turns active has stale
// maps: the blend replaces them.
public static const float PROBE_ACTIVE = 0.0;
public static const float PROBE_INACTIVE = 1.0;
public static const float PROBE_RESET = 2.0;
// The age counts the updates since the probe entered its cascade. For its first
// PROBE_SETTLE_UPDATES it moves and is classified from its fixed rays; then it keeps its
// place and state until it leaves the cascade. Left free, probes between two surfaces
// (pushed off the one below into what reads as the inside of the one above, moved back
// down through it) changed places and states every frame, and every change restarted their
// maps: 35 flips a frame on the street view, its slow change 0.61 % of the pixels.
public static const float PROBE_SETTLE_UPDATES = 8.0;

public float probe_state(float w)
{
    return w - 4.0 * floor(w * 0.25);
}

public float probe_age(float w)
{
    return floor(w * 0.25);
}

// Distances a ray stores for a miss and at most (halves).
public static const float PROBE_MISS_DISTANCE = 60000.0;

// Mirrors `GpuCascade` in crates/forge-render/src/probes.rs.
public struct ProbeCascade
{
    public int3 origin;   // the world cell of the volume's first probe (probes stand at cell centres)
    public float spacing; // metres between probes
    public int3 previous; // the origin of the frame before: the probes outside it are new this frame
    public uint pad;
};

// Mirrors `GpuProbeField` in crates/forge-render/src/probes.rs: one per frame.
public struct ProbeField
{
    public float4 rotation[3];    // rows of this frame's random rotation of the rays
    public ProbeCascade cascades[PROBE_MAX_CASCADES];
    public uint3 counts;          // probes per cascade along x, y (up) and z, each even
    public uint cascade_count;
    public float3 camera;         // world metres: the cascades fade out around it
    public uint rays;             // per probe and frame, the first PROBE_FIXED_RAYS fixed
    public uint2* ray_data;       // per probe and ray: radiance and distance as halves (`pack_ray`)
    public float4* probe_data;    // per probe: its offset from its cell's centre (xyz, metres) and state (w)
    public uint irradiance_image; // sampled indices of the two atlases
    public uint distance_image;
    public uint irradiance_storage; // their storage indices
    public uint distance_storage;
    public float2 irradiance_texel; // 1 / the atlas's size in texels
    public float2 distance_texel;
    public float hysteresis;      // the share of the maps an update keeps
    public uint pad0;
    public uint2 pad1;
};

static const float PI = 3.14159265;

// ------------------------------------------------------------------ directions ---

public float2 sign_not_zero(float2 v)
{
    return float2(v.x >= 0.0 ? 1.0 : -1.0, v.y >= 0.0 ? 1.0 : -1.0);
}

// The octahedral map of the unit direction `d` to [-1, 1]² (Cigolle et al., JCGT 2014), with
// +y at the centre: the upper hemisphere fills the inner diamond, the lower one folds into the
// corners.
public float2 oct_encode(float3 d)
{
    float2 p = d.xz / (abs(d.x) + abs(d.y) + abs(d.z));
    return d.y >= 0.0 ? p : (1.0 - abs(p.yx)) * sign_not_zero(p);
}

// The unit direction at `p` in [-1, 1]² (`oct_encode` inverted).
public float3 oct_decode(float2 p)
{
    float h = 1.0 - abs(p.x) - abs(p.y);
    float2 q = h >= 0.0 ? p : (1.0 - abs(p.yx)) * sign_not_zero(p);
    return normalize(float3(q.x, h, q.y));
}

// The direction at the centre of interior texel `t` of a map `texels` a side.
public float3 texel_direction(uint2 t, uint texels)
{
    return oct_decode((float2(t) + 0.5) / float(texels) * 2.0 - 1.0);
}

// Ray `i` of `n` spread over the sphere (the spherical Fibonacci lattice): heights evenly
// spaced, turning by the golden angle.
public float3 fibonacci_direction(uint i, uint n)
{
    float y = 1.0 - (2.0 * float(i) + 1.0) / float(n);
    float r = sqrt(max(0.0, 1.0 - y * y));
    float phi = 2.0 * PI * frac(float(i) * 0.618033989);
    return float3(cos(phi) * r, y, sin(phi) * r);
}

// The direction of a probe's ray `ray` this frame: the fixed rays are a lattice of their own;
// the others a lattice turned by the frame's rotation, so the probes see new directions every
// frame and the blend averages them.
public float3 probe_ray_direction(ProbeField* field, uint ray)
{
    if (ray < PROBE_FIXED_RAYS)
        return fibonacci_direction(ray, PROBE_FIXED_RAYS);
    float3 d = fibonacci_direction(ray - PROBE_FIXED_RAYS, field[0].rays - PROBE_FIXED_RAYS);
    return float3(dot(field[0].rotation[0].xyz, d), dot(field[0].rotation[1].xyz, d), dot(field[0].rotation[2].xyz, d));
}

// ------------------------------------------------------------------- the grid ---

// Probes live in their cascade's slots by world cell modulo the counts, so a cascade that
// scrolls with the camera keeps every probe it still covers where it was; only the planes it
// enters are new (and take the slots of the planes it left).
public uint probe_index(ProbeField* field, uint cascade, int3 cell)
{
    int3 n = int3(field[0].counts);
    int3 slot = wrap_cells(cell, n);
    return uint(((int(cascade) * n.z + slot.z) * n.y + slot.y) * n.x + slot.x);
}

// `cells` modulo `n`, in [0, n) for negative cells too. The remainder is taken of a positive
// number: a signed remainder of a negative one came out as the unsigned one on the RTX 5070
// Ti's driver (-281 % 24 gave 23 where the SPIR-V's OpSRem means -17), which put probes 16
// cells from where they were sampled. Cells stay within ±65536 counts of zero (262 km at 4 m).
public int3 wrap_cells(int3 cells, int3 n)
{
    return (cells + n * 65536) % n;
}

// Which probe `probe` is this frame.
public struct ProbeSlot
{
    public uint cascade;
    public int3 cell;       // its world cell
    public float3 position; // its cell's centre, world metres (without its offset)
    public float spacing;
    public bool fresh;      // its cell entered the cascade this frame: nothing measured yet
};

public ProbeSlot probe_slot(ProbeField* field, uint probe)
{
    int3 n = int3(field[0].counts);
    uint per_cascade = uint(n.x * n.y * n.z);
    ProbeSlot s;
    s.cascade = probe / per_cascade;
    uint local = probe % per_cascade;
    int3 slot = int3(int(local % uint(n.x)), int((local / uint(n.x)) % uint(n.y)), int(local / uint(n.x * n.y)));
    ProbeCascade c = field[0].cascades[s.cascade];
    s.cell = c.origin + wrap_cells(slot - c.origin, n);
    s.spacing = c.spacing;
    s.position = (float3(s.cell) + 0.5) * c.spacing;
    s.fresh = any(s.cell < c.previous) || any(s.cell >= c.previous + n);
    return s;
}

// Where probe `probe`'s map of `texels` a side (its tile `tile` a side, border included)
// starts in its atlas: the atlas holds a row of x × y tiles per z of every cascade.
public uint2 probe_tile(ProbeField* field, uint probe, uint tile)
{
    uint row = field[0].counts.x * field[0].counts.y;
    return uint2(probe % row, probe / row) * tile;
}

// The interior texel (0..texels-1) whose value border texel `t` of a tile holds: the
// octahedral map continues across each edge mirrored, and across each corner at the opposite
// corner, so bilinear filtering near an edge blends the directions that really neighbour it.
public uint2 border_source(uint2 t, uint texels)
{
    uint last = texels + 1;
    bool left = t.x == 0;
    bool right = t.x == last;
    bool top = t.y == 0;
    bool bottom = t.y == last;
    uint2 c = t;
    if ((left || right) && (top || bottom))
        c = uint2(left ? texels : 1u, top ? texels : 1u);
    else if (top || bottom)
        c = uint2(last - t.x, top ? 1u : texels);
    else if (left || right)
        c = uint2(left ? 1u : texels, last - t.y);
    return c - 1u;
}

// A ray's result in two words: radiance (rgb) and distance as halves; a back face stores its
// distance negated, a miss PROBE_MISS_DISTANCE.
public uint2 pack_ray(float3 radiance, float distance)
{
    float3 r = clamp(radiance, 0.0, 60000.0);
    float d = clamp(distance, -PROBE_MISS_DISTANCE, PROBE_MISS_DISTANCE);
    return uint2(f32tof16(r.x) | (f32tof16(r.y) << 16), f32tof16(r.z) | (f32tof16(d) << 16));
}

public float4 unpack_ray(uint2 v)
{
    return float4(f16tof32(v.x & 0xFFFFu), f16tof32(v.x >> 16), f16tof32(v.y & 0xFFFFu), f16tof32(v.y >> 16));
}

// ------------------------------------------------------------------- sampling ---

// How much cascade `c` counts at `p`: 1 near the camera, fading to 0 over the cell before the
// points whose eight probes (after the bias) might leave the volume. The volume's origin
// rounds the camera to its cells, so its probes always reach (counts / 2 − 1) cells from the
// camera; measuring from the camera itself (not the volume) keeps the fade still while the
// volume scrolls.
public float cascade_weight(ProbeField* field, uint c, float3 p)
{
    float s = field[0].cascades[c].spacing;
    float3 reach = (float3(field[0].counts) * 0.5 - 1.5) * s;
    float3 w = saturate((reach - abs(p - field[0].camera)) / s);
    return w.x * w.y * w.z;
}

// Irradiance at `p` (unit normal `n`, `v` towards the viewer) from cascade `c`'s eight probes
// around it, in the resolve's units (the cosine-weighted mean of the radiance the rays met, as
// the shading multiplies it by the albedo); w is 1 when at least one of them is active, else 0.
public float4 cascade_irradiance(ProbeField* field, uint c, float3 p, float3 n, float3 v)
{
    ProbeCascade cascade = field[0].cascades[c];
    float s = cascade.spacing;
    int3 counts = int3(field[0].counts);
    // The 2021 paper's bias: off the surface, mostly towards the viewer, in proportion to the
    // spacing, so the visibility test does not sit on the surface's own shadow boundary.
    float3 q = p + (n * 0.2 + v * 0.8) * (0.75 * 0.3 * s);
    float3 grid = q / s - 0.5;
    int3 base = int3(floor(grid));
    float3 alpha = grid - float3(base);
    int3 clamped = clamp(base, cascade.origin, cascade.origin + counts - 2);
    alpha = saturate(alpha + float3(base - clamped));
    base = clamped;
    // The corners' slots step from the base's, wrapping by a subtraction: `probe_index` and
    // `probe_tile` divide, and 8 probes' divisions cost the resolve more than their fetches.
    int3 base_slot = wrap_cells(base, counts);
    uint row = uint(counts.x * counts.y);
    float3 sum = float3(0.0);
    float total = 0.0;
    bool found = false;
    [unroll]
    for (uint k = 0; k < 8; k++)
    {
        int3 corner = int3(int(k & 1u), int((k >> 1) & 1u), int(k >> 2));
        int3 cell = base + corner;
        int3 slot = base_slot + corner;
        slot = select(slot >= counts, slot - counts, slot);
        uint2 tile = uint2(uint(slot.y * counts.x + slot.x), c * uint(counts.z) + uint(slot.z));
        uint probe = tile.y * row + tile.x;
        float4 data = field[0].probe_data[probe];
        if (probe_state(data.w) == PROBE_INACTIVE)
            continue;
        found = true;
        float3 position = (float3(cell) + 0.5) * s + data.xyz;
        float3 tri = max(lerp(1.0 - alpha, alpha, float3(corner)), 0.001);
        // Probes behind the surface count less, smoothly.
        float3 to_probe = normalize(position - p);
        float facing = (dot(to_probe, n) + 1.0) * 0.5;
        float w = facing * facing + 0.2;
        // Visibility: does the probe see q? Its distances towards q give a mean and a
        // variance; Chebyshev's inequality bounds the chance that q is nearer than what the
        // probe's rays met in that direction.
        float3 out_dir = q - position;
        float r = max(length(out_dir), 1e-4);
        float2 moments = sample_image_level(field[0].distance_image, SAMPLER_LINEAR_CLAMP,
            (float2(tile * DISTANCE_TILE) + 1.0 + (oct_encode(out_dir / r) * 0.5 + 0.5) * float(DISTANCE_TEXELS)) * field[0].distance_texel, 0.0).xy;
        if (r > moments.x)
        {
            float variance = abs(moments.y - moments.x * moments.x);
            float d = r - moments.x;
            float chebyshev = variance / (variance + d * d);
            w *= max(0.05, chebyshev * chebyshev * chebyshev);
        }
        w = max(w, 1e-6);
        // Weights that are already small shrink further: a faint leak reads as light in the dark.
        if (w < 0.2)
            w *= w * w / (0.2 * 0.2);
        w *= tri.x * tri.y * tri.z;
        float3 e = sample_image_level(field[0].irradiance_image, SAMPLER_LINEAR_CLAMP,
            (float2(tile * IRRADIANCE_TILE) + 1.0 + (oct_encode(n) * 0.5 + 0.5) * float(IRRADIANCE_TEXELS)) * field[0].irradiance_texel, 0.0).rgb;
        // Blend the square roots: a leak from a bright probe weighs less.
        sum += w * sqrt(max(e, 0.0));
        total += w;
    }
    if (!found || total <= 0.0)
        return float4(0.0);
    float3 root = sum / total;
    return float4(root * root, 1.0);
}

// Irradiance at `p` from the finest cascades that cover it, finer first, each over what the
// finer ones left; `open_sky` (the sky's irradiance at `n`, unoccluded) beyond them all.
public float3 probe_irradiance(ProbeField* field, float3 p, float3 n, float3 v, float3 open_sky)
{
    float3 e = float3(0.0);
    float covered = 0.0;
    for (uint c = 0; c < field[0].cascade_count && covered < 0.999; c++)
    {
        float w = cascade_weight(field, c, p);
        if (w <= 0.0)
            continue;
        float4 ce = cascade_irradiance(field, c, p, n, v);
        if (ce.w <= 0.0)
            continue;
        float share = (1.0 - covered) * w;
        e += share * ce.rgb;
        covered += share;
    }
    return e + (1.0 - covered) * open_sky;
}
