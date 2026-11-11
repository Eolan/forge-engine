// The reflection cubes as their readers see them (D-108, steps 3 and 4): the records (`GpuCube`,
// `GpuCubeSet` in crates/forge-render/src/reflection_cubes.rs), where a direction falls in a tile,
// which cubes a point is in, and a surface's light from them, for the resolve (meshlet.slang), the
// fog's froxels and the particles.
module reflection_cubes;

import bindless;
import probes;

// Mirrors `GpuCube`, `GpuCubeSet`.
public struct Cube
{
    public float4 center; // the point its faces were drawn from, in the scene frame; w: how far it has faded in
    public float4 lo;     // its lowest corner
    public float4 hi;     // its highest corner
    public uint4 tile;    // its tile, in tiles
};

public struct CubeSet
{
    public uint image;
    public uint count;
    public uint2 pad;
    public Cube cubes[32]; // `CUBE_LIST`, the nearest first
};

public static const uint CUBE_TILES = 8u;        // `reflection_cubes::CUBE_TILES`
public static const uint CUBE_TILE_TEXELS = 256u;
public static const float CUBE_LAST_LEVEL = 4.0; // the roughest that reads a reflection
public static const uint CUBE_DIFFUSE_LEVEL = 5u; // `CUBE_LEVELS` - 1: the diffuse light, by the normal
public static const float CUBE_ROUGHEST = 0.42;  // where a row stops asking for this reflection
public static const float CUBE_PAST_ITS_WALLS = 1.0; // metres a cube is read beyond its volume
public static const uint CUBE_PICKED = 3u;        // cubes blended at a point
public static const float CUBE_STREET_BLEND = 8.0; // metres over which a street cube's weight rises from its walls

// Where direction `d` falls in a tile, as fractions of the tile, the level's texel and a half kept
// clear of its edge. `cube_prefilter_main` has the same.
public float2 cube_in_tile(float3 d, uint level)
{
    float2 p = oct_encode(d) * 0.5 + float2(0.5);
    float inset = 1.0 / float(CUBE_TILE_TEXELS >> level);
    return float2(inset) + p * (1.0 - 2.0 * inset);
}

// The cubes point `p` is deepest in, up to CUBE_PICKED, and their weights: a room's before a
// street's whatever their order, so that the room it is in counts most across a wall shared by two.
// It reaches past its walls a little, for what hangs on them or stands in a doorway. A cube's
// weight rises over its first half metre inside; a street cube's over CUBE_STREET_BLEND, so that
// the lightest dropped for a fourth is light and the open sky takes over smoothly where the street
// cubes end; each by how far it has faded in (`Cube::center.w`); and a street cube's by what the
// rooms there leave it.
// `face`: a face being drawn, which counts cubes not yet whole too (they are dark), all at once.
// `cover`: how much of the point the cubes cover, from their walls and fades, not the street
// cubes' wider blend: near a street cube's walls the cube still stands for the place, not the sky.
public void cube_pick(CubeSet* set, float3 p, bool face, out uint found[CUBE_PICKED], out float weight[CUBE_PICKED], out float cover)
{
    cover = 0.0;
    float rank[CUBE_PICKED];
    for (uint k = 0u; k < CUBE_PICKED; k++)
    {
        found[k] = set[0].count;
        weight[k] = 0.0;
        rank[k] = 0.0;
    }
    for (uint i = 0u; i < set[0].count; i++)
    {
        Cube c = set[0].cubes[i];
        if (c.tile.z == 0u && !face)
            continue;
        float3 inside = min(p - c.lo.xyz, c.hi.xyz - p) + float3(CUBE_PAST_ITS_WALLS);
        float edge = min(inside.x, min(inside.y, inside.z));
        bool room = c.tile.w != 0u;
        // A street cube fades to the open sky over its outer metres: no line where it ends.
        float covers = saturate(edge / 0.5) * (room ? 1.0 : saturate(edge / CUBE_STREET_BLEND)) * (face ? 1.0 : c.center.w);
        cover = max(cover, covers);
        float w = covers;
        if (w <= 0.0)
            continue;
        float here = w + (room ? 1e4 : 0.0);
        // In among those kept, the heaviest first.
        uint at = CUBE_PICKED;
        for (uint k = 0u; k < CUBE_PICKED; k++)
        {
            if (at == CUBE_PICKED && here > rank[k])
                at = k;
        }
        if (at == CUBE_PICKED)
            continue;
        for (uint k = CUBE_PICKED - 1u; k > at; k--)
        {
            found[k] = found[k - 1u];
            weight[k] = weight[k - 1u];
            rank[k] = rank[k - 1u];
        }
        found[at] = i;
        weight[at] = w;
        rank[at] = here;
    }
    // Inside a room its cube is the light: the street cubes' views from outside count only as far
    // as the rooms leave room for them (they rank after the rooms, so the rooms are all in front).
    float room_cover = 0.0;
    for (uint k = 0u; k < CUBE_PICKED; k++)
    {
        if (found[k] < set[0].count && set[0].cubes[found[k]].tile.w != 0u)
            room_cover = max(room_cover, weight[k]);
    }
    for (uint k = 0u; k < CUBE_PICKED; k++)
    {
        if (found[k] < set[0].count && set[0].cubes[found[k]].tile.w == 0u)
            weight[k] *= 1.0 - room_cover;
    }
}

// A room's light from its cube (D-108, step 4): what the cubes of `set` hold for a surface at `p`
// (the scene frame) with normal `n`, rgb in the units of the sky's irradiance; a: how much of the point they cover. A
// cube's own faces, drawn before it has any light, count a cube not yet whole as dark: its first
// round is the direct light alone, and the others build on it.
public float4 cube_ambient_in(CubeSet* set, float3 p, float3 n, bool face)
{
    if (set == nullptr)
        return float4(0.0);
    uint found[CUBE_PICKED];
    float weight[CUBE_PICKED];
    float cover;
    cube_pick(set, p, face, found, weight, cover);
    float3 sum = float3(0.0);
    float total = 0.0;
    for (uint k = 0u; k < CUBE_PICKED; k++)
    {
        if (found[k] >= set[0].count)
            continue;
        float w = weight[k];
        Cube c = set[0].cubes[found[k]];
        if (c.tile.z != 0u)
        {
            float2 uv = (float2(c.tile.xy) + cube_in_tile(n, CUBE_DIFFUSE_LEVEL)) / float(CUBE_TILES);
            sum += w * g_sampled_images[NonUniformResourceIndex(set[0].image)].SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], uv, float(CUBE_DIFFUSE_LEVEL)).rgb;
        }
        total += w;
    }
    return float4(total > 0.0 ? sum / total : float3(0.0), total > 0.0 ? cover : 0.0);
}
