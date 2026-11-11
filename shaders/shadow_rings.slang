// The sun's shadow rings as their readers see them (D-108): the rings' records (`GpuShadowRing` and
// `GpuRingSet` in crates/forge-render/src/shadow_rings.rs), the filter's tap and disc, and a point's
// share of the sun for what has no surface: particles, splashes, the dust's shafts, the far plants'
// impostors and the fog's froxels. The resolve's own filter, with its blocker search and its
// receiver plane, stays in meshlet.slang (`ring_query`).
module shadow_rings;

import bindless;

public static const uint RING_COUNT = 5;
public static const float RING_TEXELS = 2048.0;
public static const float GOLDEN_ANGLE = 2.39996323;

public struct ShadowRing
{
    public float4 axis_u;   // across the sun, and what to add to a dot with it for metres across the ring
    public float4 axis_v;
    public float4 axis_s;   // towards the sun, and what to add for metres over the bottom of its depth
    public float texel;     // metres
    public float inv_width;
    public float inv_range;
    public uint image;      // sampled index of its composed layer, or 0xFFFFFFFF: not ready
};

public struct RingSet
{
    public ShadowRing rings[RING_COUNT];
    public float tan_radius;   // of the sun's disc
    public uint search_taps;
    public uint filter_taps;
    public float max_radius;   // texels
    public float search_reach; // metres along the sun: the widest penumbra the blocker search looks for
    public float bias;         // texels
    public float normal_offset;// texels
    public uint contact_steps; // the contact march's (`ring_contact`), 0: none
    public float contact_reach;    // its length, the ring's texels
    public float contact_thickness;// how far behind what the depth buffer shows a step counts, metres
    public float contact_far;      // how far from the camera it goes on, metres
};

// One percentage-closer tap with bilinear weights: `at` in texels, lit where the stored height is
// not over `z`.
public float ring_tap(uint image, float2 at, float z)
{
    float2 corner = floor(at - 0.5);
    float2 t = at - 0.5 - corner;
    float4 g = g_sampled_images_r[NonUniformResourceIndex(image)].GatherRed(g_samplers[SAMPLER_NEAREST_CLAMP], (corner + 1.0) / RING_TEXELS);
    float4 c = float4(g.x <= z ? 1.0 : 0.0, g.y <= z ? 1.0 : 0.0, g.z <= z ? 1.0 : 0.0, g.w <= z ? 1.0 : 0.0);
    float top = c.w + (c.z - c.w) * t.x;
    float bottom = c.x + (c.y - c.x) * t.x;
    return top + (bottom - top) * t.y;
}

// A disc's `k`-th of `n` points, `radius` wide, turned by `turn`.
public float2 ring_disc(uint k, uint n, float radius, float turn)
{
    float r = sqrt((float(k) + 0.5) / float(n)) * radius;
    float a = float(k) * GOLDEN_ANGLE + turn;
    return float2(cos(a), sin(a)) * r;
}

// Taps of `ring_sun`'s filter, and its radius in texels: a soft edge for what has no surface.
static const uint RING_SUN_TAPS = 4u;
static const float RING_SUN_RADIUS = 1.5;

// The share of the sun a point sees that has no surface (D-108 step 5): `relative` to the camera
// the rings were drawn for, `turn` a noise in [0, 1) that turns the filter's taps. The finest ring
// that holds it, a few bilinear taps round it; the sun is seen past the last ring, and where the
// rings are not given (`set` null), as the resolve's pixels take it.
public float ring_sun(RingSet* set, float3 relative, float turn)
{
    if (set == nullptr)
        return 1.0;
    for (uint i = 0u; i < RING_COUNT; i++)
    {
        ShadowRing r = set[0].rings[i];
        if (r.image == 0xFFFFFFFFu)
            continue;
        float2 uv = float2(dot(r.axis_u.xyz, relative) + r.axis_u.w, dot(r.axis_v.xyz, relative) + r.axis_v.w) * r.inv_width;
        float margin = (RING_SUN_RADIUS + 2.0) / RING_TEXELS;
        if (min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y)) < margin)
            continue;
        float z = (dot(r.axis_s.xyz, relative) + r.axis_s.w + r.texel * set[0].bias) * r.inv_range;
        // Under the ring's depth range nothing is known of it.
        if (z < 0.0)
            return 1.0;
        float2 at = uv * RING_TEXELS;
        float lit = 0.0;
        for (uint k = 0u; k < RING_SUN_TAPS; k++)
            lit += ring_tap(r.image, at + ring_disc(k, RING_SUN_TAPS, RING_SUN_RADIUS, turn * 6.2831853), z);
        return lit / float(RING_SUN_TAPS);
    }
    return 1.0;
}
