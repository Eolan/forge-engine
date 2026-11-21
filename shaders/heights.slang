// The heights seen from above as their readers see them (D-108 step 6): the static casters' highest
// point in each texel of the rings' tiles, their axis straight up, in two levels round the camera
// (`GpuHeightLevel`, `GpuHeightSet` in crates/forge-render/src/shadow_rings.rs), and a march against
// them. Two and a half dimensions: a column stands solid from its top down to the ground.
module heights;

import bindless;

public static const uint HEIGHT_LEVELS = 2;
public static const float HEIGHT_TEXELS = 2048.0;
// How far a march that starts under the heights (under an eave, a balcony) may go before what is
// over it counts, metres: out from under a ledge it sees as any other.
public static const float HEIGHT_GRACE = 1.5;

public struct HeightLevel
{
    public float2 window; // metres to add to a relative point's (z, x) for its place across the level's window
    public float2 wrap;   // metres to add to that place for its place in the wrapped image
    public float bottom;  // metres to add to a relative point's y for its height over the bottom of the range
    public float texel;   // metres
    public float width;   // metres
    public uint image;    // sampled index, or 0xFFFFFFFF: not drawn
};

public struct HeightSet
{
    public HeightLevel levels[HEIGHT_LEVELS];
    public float range; // the metres of height an image's 0 to 1 hold
    public float3 pad;
};

// The top of the column `relative` stands in (relative to the camera the levels were drawn for, as
// its y), from level `l`; false where the level does not hold the point.
bool height_in(HeightLevel l, float range, float3 relative, out float top)
{
    top = -1e30;
    if (l.image == 0xFFFFFFFFu)
        return false;
    float2 across = relative.zx + l.window;
    if (any(across < float2(l.texel)) || any(across >= float2(l.width - l.texel)))
        return false;
    float2 wrapped = frac((across + l.wrap) / l.width) * HEIGHT_TEXELS;
    int2 texel = min(int2(wrapped), int2(int(HEIGHT_TEXELS) - 1));
    top = g_sampled_images_r[NonUniformResourceIndex(l.image)].Load(int3(texel, 0)) * range - l.bottom;
    return true;
}

// The top of the column `relative` stands in, from the finest level at or past `from` that holds
// it, and that level's texel; false where none does.
public bool height_over(HeightSet* set, float3 relative, uint from, out float top, out float texel)
{
    for (uint i = from; i < HEIGHT_LEVELS; i++)
    {
        if (height_in(set[0].levels[i], set[0].range, relative, top))
        {
            texel = set[0].levels[i].texel;
            return true;
        }
    }
    top = -1e30;
    texel = 1.0;
    return false;
}

public struct HeightHit
{
    public bool blocked;
    public float3 at; // where it met them, on the open side, relative as the march's origin
    public float3 n;  // the face it met: a wall facing back along the march, or a top facing up
};

// Marches from `origin` along the unit `d` against the heights, in `steps` steps that lengthen
// with the distance out to `reach` metres, the first `jitter` (0 to 1) of a step on. A step's level
// comes from its index: the first half read the fine level where it holds them. Open while it
// stays over the heights, and past what the levels hold; it ends where it first goes under them,
// the crossing then halved four times.
public HeightHit height_march(HeightSet* set, float3 origin, float3 d, float reach, uint steps, float jitter)
{
    HeightHit hit;
    hit.blocked = false;
    hit.at = origin;
    hit.n = float3(0.0, 1.0, 0.0);
    float top;
    float texel;
    if (!height_over(set, origin, 0u, top, texel))
        return hit;
    float grace = origin.y < top ? max(HEIGHT_GRACE, 6.0 * texel) : 0.0;
    float first = 2.0 * texel;
    float growth = pow(max(reach / first, 1.0), 1.0 / float(max(steps, 1u)));
    float open_to = 0.0;
    float t = first * pow(growth, jitter);
    for (uint k = 0u; k < steps; k++)
    {
        float3 p = origin + d * t;
        uint level = k * 2u < steps ? 0u : 1u;
        if (!height_over(set, p, level, top, texel))
            return hit;
        if (p.y >= top)
        {
            grace = 0.0;
            open_to = t;
        }
        else if (t >= grace)
        {
            float under = t;
            for (uint b = 0u; b < 4u; b++)
            {
                float middle = 0.5 * (open_to + under);
                float3 q = origin + d * middle;
                float q_top;
                float q_texel;
                if (height_over(set, q, level, q_top, q_texel) && q.y < q_top)
                {
                    under = middle;
                    top = q_top;
                    texel = q_texel;
                }
                else
                    open_to = middle;
            }
            float3 last_open = origin + d * open_to;
            float3 first_under = origin + d * under;
            hit.blocked = true;
            if (d.y < 0.0 && last_open.y >= top)
            {
                // Down through a column's top: the ground, or a roof.
                hit.at = float3(first_under.x, top + 0.5 * texel, first_under.z);
            }
            else
            {
                float2 flat = d.xz;
                float flat_length = length(flat);
                hit.n = flat_length > 0.2 ? float3(-flat.x, 0.0, -flat.y) / flat_length : -d;
                hit.at = last_open;
            }
            return hit;
        }
        t *= growth;
    }
    return hit;
}
