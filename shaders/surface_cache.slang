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
// than the hash's slots, and are made at the next frame's start from the least of the frame's
// claims on their slot, not by the first lookup to ask, so that two runs match (D-016). The
// cache is read at every hit, whatever the ray's length (Solari reads
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
// A slot with no entry.
public static const uint CACHE_EMPTY = 0xFFFFFFFFu;
// A slot no lookup claimed this frame (`cache_claim`).
public static const uint64_t CACHE_UNCLAIMED = 0xFFFFFFFFFFFFFFFFull;
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
    public int4 last_origins[CACHE_LEVELS]; // the last frame's, whose lookups made the claims
    public float3 centre;              // the levels' centre, the camera: scene metres
    public float cell;                 // the finest cell, metres
    public CacheEntry* entries;
    public uint* index;                // CACHE_LEVELS × CACHE_LEVEL_SLOTS entries' numbers
    public uint* free;                 // [0]: how many entries are free; then their numbers
    public uint* used;                 // [0]: the entries in use after the blend (host-read)
    public uint64_t* claims;           // per slot, the least claim of the frame (`cache_claim`)
    public uint* claimed;              // [0]: the slots claimed this frame; then their numbers
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
    public float size;  // the level's cell, metres
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
    k.size = size;
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

// A free entry's number, taken off the stack; CACHE_EMPTY when none is left. Only
// `cache_make` takes, in a pass of its own: nothing frees one meanwhile, and a thread that
// counts past the stack's end puts its count back.
uint cache_take(SurfaceCacheField* c)
{
    uint was;
    InterlockedAdd(c[0].free[0], 0xFFFFFFFFu, was);
    if (was == 0u || was > c[0].pool)
    {
        InterlockedAdd(c[0].free[0], 1u, was);
        return CACHE_EMPTY;
    }
    return c[0].free[was];
}

// ------------------------------------------------------------------- the claims ---

// A lookup that finds no entry in its slot claims it for its point, and the next frame's
// `cache_make` makes the entry from the slot's least claim. Two runs then make the same entries,
// whatever order the threads come in (D-016): the first point to ask, as Solari's and kajiya's
// caches take, and the free entry it came by, followed the threads, and the frames of two
// identical runs differed in 7.6 % of the classroom's pixels. Once, when the entry is made: no
// vote moves it after (D-088).
//
// A claim's 64 bits: a 16-bit hash of the rest, above it, so that the least is one of the
// frame's points picked by their own value, not the corner one; then the point, from CLAIM_LOW
// to CLAIM_LOW + CLAIM_SPAN cells off its cell's corner along each axis in 11 bits each (the
// jitter and the half cell along the normal keep it within that), and its normal's octahedral
// pair in 7 bits each (1.4 degrees).
static const float CLAIM_LOW = -1.5;
static const float CLAIM_SPAN = 4.0;
static const float CLAIM_STEPS = 2048.0;
static const float CLAIM_NORMAL_STEPS = 128.0;

uint claim_hash(uint x)
{
    x ^= x >> 16;
    x *= 0x7FEB352Du;
    x ^= x >> 15;
    x *= 0x846CA68Bu;
    x ^= x >> 16;
    return x;
}

float2 claim_sign(float2 v)
{
    return float2(v.x >= 0.0 ? 1.0 : -1.0, v.y >= 0.0 ? 1.0 : -1.0);
}

// The octahedral pair of `d` in [-1, 1]² (as `oct_encode` in probes.slang).
float2 claim_oct(float3 d)
{
    float2 p = d.xz / (abs(d.x) + abs(d.y) + abs(d.z));
    return d.y >= 0.0 ? p : (1.0 - abs(p.yx)) * claim_sign(p);
}

float3 claim_unoct(float2 p)
{
    float h = 1.0 - abs(p.x) - abs(p.y);
    float2 q = h >= 0.0 ? p : (1.0 - abs(p.yx)) * claim_sign(p);
    return normalize(float3(q.x, h, q.y));
}

uint64_t cache_claim(float3 p, float3 n, CacheCell k)
{
    float3 at = (p / k.size - float3(k.cell) - CLAIM_LOW) / CLAIM_SPAN;
    uint3 q = uint3(clamp(at * CLAIM_STEPS, 0.0, CLAIM_STEPS - 1.0));
    uint2 m = uint2(clamp((claim_oct(n) * 0.5 + 0.5) * CLAIM_NORMAL_STEPS, 0.0, CLAIM_NORMAL_STEPS - 1.0));
    uint64_t rest = (uint64_t(q.x) << 37) | (uint64_t(q.y) << 26) | (uint64_t(q.z) << 15) | (uint64_t(m.x) << 8) | (uint64_t(m.y) << 1);
    uint h = claim_hash(uint(rest) ^ claim_hash(uint(rest >> 32)));
    return (uint64_t(h >> 16) << 48) | rest;
}

// How far a claim's point may stand from the point that claimed it, metres, in a cell `size`
// across: the entry's rays start that much further off it (`surface_cache_trace_main`).
public float cache_point_error(float size)
{
    return 0.002 * size;
}

// The light the cache holds round the surface point `p` facing `n`: rgb, and w its samples (0:
// none yet). Read, an entry lives CACHE_LIFE frames more; with `make`, a point whose slot has no
// entry claims it, and the entry is made at the next frame's start. `u` jitters the point
// (`cache_cell`).
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
    if (make)
    {
        // The slot's first claim of the frame lists it, once: `cache_make` walks the list.
        uint64_t was;
        InterlockedMin(c[0].claims[k.slot], cache_claim(p, n, k), was);
        if (was == CACHE_UNCLAIMED)
        {
            uint at;
            InterlockedAdd(c[0].claimed[0], 1u, at);
            c[0].claimed[1u + at] = k.slot;
        }
    }
    return float4(0.0);
}

// Makes the entry of slot `slot` from its least claim of the last frame, where it has none and a
// free entry is left (else the slot is claimed again), and clears the claim
// (`surface_cache_make_main`, before `surface_cache_age_main`). The claim's cell is the one the
// slot stood for in the last frame's levels.
public void cache_make(SurfaceCacheField* c, uint slot)
{
    uint64_t key = c[0].claims[slot];
    if (key == CACHE_UNCLAIMED)
        return;
    c[0].claims[slot] = CACHE_UNCLAIMED;
    if (c[0].index[slot] != CACHE_EMPTY || c[0].reset != 0u)
        return;
    uint level = slot / CACHE_LEVEL_SLOTS;
    uint at = (slot % CACHE_LEVEL_SLOTS) / CACHE_FACINGS;
    int3 wrapped = int3(int(at % CACHE_CELLS), int((at / CACHE_CELLS) % CACHE_CELLS), int(at / (CACHE_CELLS * CACHE_CELLS)));
    int3 first = c[0].last_origins[level].xyz;
    int3 cell = first + ((wrapped - first) & int(CACHE_CELLS - 1u));
    uint id = cache_take(c);
    if (id == CACHE_EMPTY)
        return;
    float size = c[0].cell * float(1u << level);
    uint3 q = uint3(uint(key >> 37) & 2047u, uint(key >> 26) & 2047u, uint(key >> 15) & 2047u);
    float2 m = float2(float(uint(key >> 8) & 127u), float(uint(key >> 1) & 127u));
    CacheEntry entry;
    entry.position = (float3(cell) + (float3(q) + 0.5) / CLAIM_STEPS * CLAIM_SPAN + CLAIM_LOW) * size;
    entry.slot = slot;
    entry.normal = claim_unoct((m + 0.5) / CLAIM_NORMAL_STEPS * 2.0 - 1.0);
    entry.life = CACHE_LIFE;
    entry.cell = int4(cell, int(level));
    entry.irradiance = float4(0.0);
    // Measured from this frame's rays on (`surface_cache_age_main` takes it as alive).
    entry.fresh = float3(-1.0);
    entry.pad = 0u;
    c[0].entries[id] = entry;
    c[0].index[slot] = id;
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
