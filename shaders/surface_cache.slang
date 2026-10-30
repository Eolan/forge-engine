// D-088's surface cache: the light round the scene's surfaces, measured on them, read where the
// bounce's rays (`diffuse_trace_main`) and the cache's own rays (`surface_cache_trace_main`, both
// in meshlet.slang) meet a surface, in place of the probes' light (whose probes float in the air
// and see more sky than a surface does). The passes that age and blend the entries are in
// surface_cache_update.slang.
//
// Ported from two open-source caches, taken under the Apache License 2.0 (both are offered
// under MIT or Apache-2.0; the licences are shaders/third-party/Bevy-LICENSE-APACHE.txt and
// shaders/third-party/kajiya-LICENSE-APACHE.txt):
// - Bevy Solari's world cache, by the Bevy contributors (Bevy 0.20.1, commit a0577b5c,
//   crates/bevy_solari/src/realtime/world_cache_query.wesl, world_cache_update.wesl,
//   world_cache_compact.wesl): entries that live while they are read, one cosine ray an entry a
//   frame whose hit reads the cache, their samples blended up to a most, and the lookup jittered
//   in the surface's plane (Solari's, after Tom Clabault's notes on ReGIR, 2025).
// - kajiya's irradiance cache, Copyright (c) 2019 Embark Studios (commit 9fdec0f0,
//   assets/shaders/ircache/ircache_grid.hlsl): the dense index, levels of 32³ cells round the
//   camera each twice the last, a point's level from its distance, and the cell moved half a
//   cell along the normal.
// Forge's changes: rewritten in Slang on Forge's buffers. Solari's hashed key, checksum and
// linear probing are replaced by kajiya's dense index, whose cells wrap in place as the camera
// moves, with one slot a cell per facing. Entries come from a free list (kajiya's pool) rather
// than the hash's slots. The cache is read at every hit, whatever the ray's length (Solari reads
// it only past √3 cells). The hits are lit by Forge's own light (no light tiles), and no
// kajiya voting or resampling is taken. Solari's blend cuts an entry's samples back where its
// luminance changes; with one ray an entry a frame the change is the rays' noise, and it took
// the village's landing from 20 changing pixels a frame pair to 319, so Forge blends a plain
// average of the last samples. D-088 has the design, the measures and the patents.
module surface_cache;

import exact;

public static const uint CACHE_LEVELS = 12;
// A level's cells along each axis, a power of two (the wrap is a mask).
public static const uint CACHE_CELLS = 32;
// A cell's slots: one per facing, the normal's main axis and its sign.
public static const uint CACHE_FACINGS = 6;
public static const uint CACHE_LEVEL_SLOTS = CACHE_CELLS * CACHE_CELLS * CACHE_CELLS * CACHE_FACINGS;
// A slot with no entry; one being given an entry (for the moment between the two).
public static const uint CACHE_EMPTY = 0xFFFFFFFFu;
public static const uint CACHE_TAKING = 0xFFFFFFFEu;
// The frames an entry lives unread (Solari's `WORLD_CACHE_CELL_LIFETIME`).
public static const uint CACHE_LIFE = 10u;
// Cells kept clear at a level's edge: a point the level takes stays inside its window after the
// half cell along its normal, wherever the camera sits in its own cell (kajiya keeps one).
static const uint CACHE_EDGE = 2u;

public struct CacheEntry
{
    public float3 position;   // the surface point that made it, where its rays start: scene metres
    public uint slot;         // its slot in the index; CACHE_EMPTY while it is free
    public float3 normal;     // that point's
    public uint life;         // the frames it has left unread
    public int4 cell;         // xyz: its cell in its level; w: the level
    public float4 irradiance; // rgb: its light, in the resolve's units (the probes'); w: samples blended
    public float3 fresh;      // this frame's sample (`surface_cache_trace_main`); negative: none to
                              // take, the entry made after `surface_cache_age_main` (or free)
    public uint pad;
};

public struct SurfaceCacheField
{
    public int4 origins[CACHE_LEVELS]; // xyz: each level's first cell this frame
    public float3 centre;              // the levels' centre, the camera: scene metres
    public float cell;                 // the finest cell, metres
    public CacheEntry* entries;
    public uint* index;                // CACHE_LEVELS × CACHE_LEVEL_SLOTS entries' numbers
    public uint* free;                 // [0]: how many entries are free; then their numbers
    public uint* used;                 // [0]: the entries in use after the blend (host-read)
    public uint pool;                  // entries
    public uint frame;                 // turns the rays and the jitter
    public float most_samples;         // Solari's `world_cache_max_temporal_samples`
    public uint reset;                 // 1: every entry is freed (`SurfaceCache::reset`)
};

// The level of a point `local` metres from the levels' centre: the finest whose window, less its
// edge, holds it (kajiya's `ws_local_pos_to_cascade_idx`).
uint cache_level(SurfaceCacheField* c, float3 local)
{
    float3 cells = abs(local) / c[0].cell;
    float far = max(cells.x, max(cells.y, cells.z));
    float level = ceil(log2(max(far, 1e-6) / float(CACHE_CELLS / 2u - CACHE_EDGE)));
    return uint(clamp(level, 0.0, float(CACHE_LEVELS - 1u)));
}

uint cache_facing(float3 n)
{
    float3 a = abs(n);
    if (a.x >= a.y && a.x >= a.z)
        return n.x >= 0.0 ? 0u : 1u;
    if (a.y >= a.z)
        return n.y >= 0.0 ? 2u : 3u;
    return n.z >= 0.0 ? 4u : 5u;
}

public struct CacheCell
{
    public int3 cell;   // in its level
    public uint level;
    public uint slot;   // in the index
    public bool inside; // within its level's window (always, but for points past the last level)
};

// The cell and slot of the surface point `p` facing `n` (scene metres). `u`, two numbers in
// [0, 1), moves the point within its cell's size in the surface's plane (Solari's
// `jitter_world_position_tangent_plane`), so the cells' edges blur; (0.5, 0.5) leaves it.
public CacheCell cache_cell(SurfaceCacheField* c, float3 p, float3 n, float2 u)
{
    float size = c[0].cell * float(1u << cache_level(c, p - c[0].centre));
    float3 t = abs(n.y) < 0.999 ? normalize(cross(n, float3(0.0, 1.0, 0.0))) : float3(1.0, 0.0, 0.0);
    float3 b = cross(n, t);
    float2 o = (u - 0.5) * size;
    float3 q = p + o.x * t + o.y * b;
    CacheCell k;
    k.level = cache_level(c, q - c[0].centre);
    size = c[0].cell * float(1u << k.level);
    // Half a cell along the normal (kajiya's `IRCACHE_USE_NORMAL_BASED_CELL_OFFSET`): a thin
    // wall's two faces fall in different cells more often.
    q += n * (0.5 * size);
    k.cell = int3(floor(q / size));
    int3 local = k.cell - c[0].origins[k.level].xyz;
    k.inside = all(local >= 0) && all(local < int(CACHE_CELLS));
    uint3 wrapped = uint3(k.cell & int(CACHE_CELLS - 1u));
    k.slot = k.level * CACHE_LEVEL_SLOTS + ((wrapped.z * CACHE_CELLS + wrapped.y) * CACHE_CELLS + wrapped.x) * CACHE_FACINGS + cache_facing(n);
    return k;
}

// A free entry's number, taken off the stack; CACHE_EMPTY when none is left (or the stack was
// too busy this time: the point asks again at its next lookup).
uint cache_take(SurfaceCacheField* c)
{
    uint n = c[0].free[0];
    for (uint tries = 0u; tries < 32u && n > 0u; tries++)
    {
        uint was;
        InterlockedCompareExchange(c[0].free[0], n, n - 1u, was);
        if (was == n)
            return c[0].free[n];
        n = was;
    }
    return CACHE_EMPTY;
}

// The light the cache holds round the surface point `p` facing `n`: rgb, and w its samples (0:
// none yet). Read, an entry lives CACHE_LIFE frames more; with `make`, a point whose slot has no
// entry gets one, measured from the next frame on. `u` jitters the point (`cache_cell`).
public float4 cache_lookup(SurfaceCacheField* c, float3 p, float3 n, float2 u, bool make)
{
    CacheCell k = cache_cell(c, p, n, u);
    if (!k.inside)
        return float4(0.0);
    uint e = c[0].index[k.slot];
    if (e < c[0].pool)
    {
        if (c[0].entries[e].life != CACHE_LIFE)
            c[0].entries[e].life = CACHE_LIFE;
        return c[0].entries[e].irradiance;
    }
    if (!make || e != CACHE_EMPTY)
        return float4(0.0);
    uint was;
    InterlockedCompareExchange(c[0].index[k.slot], CACHE_EMPTY, CACHE_TAKING, was);
    if (was != CACHE_EMPTY)
        return float4(0.0);
    uint id = cache_take(c);
    if (id == CACHE_EMPTY)
    {
        c[0].index[k.slot] = CACHE_EMPTY;
        return float4(0.0);
    }
    CacheEntry entry;
    entry.position = p;
    entry.slot = k.slot;
    entry.normal = n;
    entry.life = CACHE_LIFE;
    entry.cell = int4(k.cell, int(k.level));
    entry.irradiance = float4(0.0);
    // No sample yet: made by a ray of this frame's `surface_cache_trace_main`, it is not blended.
    entry.fresh = float3(-1.0);
    entry.pad = 0u;
    c[0].entries[id] = entry;
    // The entry before its number: another lookup that finds the number reads this entry, not
    // the one the number held before. A reader that still sees the old one finds it cleared
    // (`surface_cache_age_main`): no light, the probes'.
    DeviceMemoryBarrier();
    c[0].index[k.slot] = id;
    return float4(0.0);
}

// Two numbers in [0, 1) from `seed`: a lookup's jitter.
public float2 cache_jitter(uint seed)
{
    uint a = seed * 0x9E3779B9u;
    a ^= a >> 16;
    a *= 0x85EBCA6Bu;
    a ^= a >> 13;
    uint b = a * 0xC2B2AE35u;
    b ^= b >> 16;
    return float2(float(a >> 8), float(b >> 8)) * (1.0 / 16777216.0);
}

// An entry's sample blended into its light: the plain average of its samples until it holds
// `most`, then an exponential average weighing each new one 1 / `most`.
public float4 cache_blend(float4 old, float3 fresh, float most)
{
    float count = min(old.w + 1.0, most);
    return float4(lerp_exact(old.rgb, fresh, 1.0 / count), count);
}
