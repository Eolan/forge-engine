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
import exact;

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
// A cascade's probe in a room whose own grid lights it whole (D-078): not traced, blended or
// sampled until the grid fades out, when it starts over young.
public static const float PROBE_RETIRED = 3.0;
// The age counts the updates since the probe entered its cascade. For its first
// PROBE_SETTLE_UPDATES it moves and is classified from its fixed rays; then it keeps its
// place and state until it leaves the cascade. Left free, probes between two surfaces
// (pushed off the one below into what reads as the inside of the one above, moved back
// down through it) changed places and states every frame, and every change restarted their
// maps: 35 flips a frame on the street view, its slow change 0.61 % of the pixels.
// The state pass writes a settled probe's age as PROBE_SETTLE_UPDATES + 1, so the blend,
// which runs after it, tells a probe that has just settled (its last update as a young
// one) from one settled before (`probe_due`).
public static const float PROBE_SETTLE_UPDATES = 8.0;

public float probe_state(float w)
{
    return w - 4.0 * floor(w * 0.25);
}

public float probe_age(float w)
{
    return floor(w * 0.25);
}

// Whether a probe's maps light anything: active, or active again (reset); not inactive or retired.
public bool probe_lights(float w)
{
    float s = probe_state(w);
    return s == PROBE_ACTIVE || s == PROBE_RESET;
}

// Distances a ray stores for a miss and at most (halves).
public static const float PROBE_MISS_DISTANCE = 60000.0;
// The trilinear weight under which a probe is left out of a lookup (#70), fading in up to twice
// it: its fetches cost as much as a heavy probe's for a share of the light under 5 %.
static const float PROBE_LEAST_WEIGHT = 0.05;
// The same in front of the surface, in a cascade whose probes are at most PROBE_ROOM_SPACING
// apart, room-sized (the village's 1 m, the models lab's rooms; 2026-10-17, the lookup's
// comment): 0.14–0.23 ms in the village. Coarser cascades keep #70's: there the probes in front
// cost 0.1 ms of the city's shading and 0.09 of its probes' rays, for light no room's corner
// shows.
static const float PROBE_LEAST_FRONT = 0.01;
public static const float PROBE_ROOM_SPACING = 1.5;
// How many of a room-sized cascade's spacings, inside its edge, its light takes to blend into
// what lies beyond it on a room's surfaces (#334).
static const float ROOM_BLEND = 4.0;
// ROOM_BLEND across the ground, one spacing up and down, where the cascades reach a few metres.
static const float3 ROOM_BAND = float3(ROOM_BLEND, 1.0, ROOM_BLEND);
// In a room (D-068), the share of the trilinear weight its own probes must hold among the eight
// for a cascade to claim all of a point's light; with less, the next cascade fills the rest.
static const float PROBE_ROOM_COVER = 0.25;

// Mirrors `GpuCascade` in crates/forge-render/src/probes.rs.
public struct ProbeCascade
{
    public int3 origin;   // the world cell of the volume's first probe (probes stand at cell centres)
    public float spacing; // metres between probes
    public int3 previous; // the origin of the frame before: the probes outside it are new this frame
    public uint pad;
    public uint3 counts;  // its probes along x, y (up) and z, each even (the finest may reach farther, #353)
    public uint first_row; // its first row of the atlases: a row of x × y probes per z
};

// ------------------------------------------------------------------- regions ---
//
// D-068 (research/probe-leaks-indoors.md): a scene that knows its rooms (the village, from its
// generator) gives each a region number, and a probe lights only the surfaces of its own
// region. A thin partition passed the visibility test against a sunlit room behind it (a tinted
// blotch), and a sunlit probe outside saw a window's head through the opening (a lit cap):
// neither is a failure of the test, so a second test decides. Every surface outside every
// volume is outside (REGION_OUTSIDE), as are the probes there.

// Mirrors `GpuRegionVolume` in crates/forge-render/src/probes/regions.rs.
public struct RegionVolume
{
    public float2 corners[4]; // its outline on the ground, x and z in the scene frame, anticlockwise from above (x right, z down)
    public float floor;       // its extent up, metres in the scene frame
    public float ceiling;
    public uint region;       // the room it belongs to: on a doorway, the side of corners 0–1
    public uint other;        // a doorway's other side, at corners 3–2; else `region`
    public uint kind;         // REGION_ROOM, REGION_RECESS (a window's inner reveal: its room's), REGION_DOORWAY or REGION_SOLID
    public uint pad;          // (a doorway's split is the frame's: `ProbeField::region_splits`)
};

// Mirrors `GpuRegionSet` in regions.rs (#338): a set of volumes (a streamed cell's) and the grid
// over its box listing them.
public struct RegionSet
{
    public float3 origin; // its grid's corner at its lowest floor, scene frame: its volumes are relative to it
    public uint cells;    // its first cell in `region_cells`
    public uint2 dims;    // its cells along x and z
    public uint2 pad;
};

// Mirrors `GpuTopEntry` in regions.rs: a coarse cell of `region_top_cell` metres, the first of
// the sets reaching it in `region_top_list` and their count (0: an empty entry).
public struct RegionTop
{
    public int2 key;
    public uint first;
    public uint count;
};

public static const uint REGION_OUTSIDE = 0u;
public static const uint REGION_NONE = 0xFFFFFFFFu; // a probe in a recess or a doorway: no region's
public static const uint REGION_ROOM = 0u;
public static const uint REGION_RECESS = 1u;
public static const uint REGION_DOORWAY = 2u;
public static const uint REGION_SOLID = 3u; // a house's body outside its rooms: its walls, its slabs; no probe's, lit by all

// What `region_at` found: a region, a doorway's other side and how far across it the point
// stands (0 at `region`'s face, 1 at `other`'s), and the volume's kind.
public struct RegionHit
{
    public uint region;
    public uint other;
    public float ramp;
    public uint kind;
};

// ------------------------------------------------------------------ room grids ---
//
// D-078 (research/probe-levels-indoors.md §2.1): a room near the camera gets a probe grid of its
// own, fixed in the world, its outer probes 0.35 m in from its walls, floor and ceiling
// (rooms.rs). Its surfaces and the probes' hits in it take their light from it alone, faded in
// over the cascades' once its probes have settled.

public static const uint ROOM_NONE = 0xFFFFFFFFu;
// `ProbeSlot::cascade` of a room grid's probe.
public static const uint ROOM_CASCADE = PROBE_MAX_CASCADES;
// Metres between a grid's outer probes and its room's walls (`INSET` in rooms.rs).
static const float ROOM_INSET = 0.35;
// Metres past a room's box within which a point in a house's body takes the room's grid: a wall's
// thickness and more (`room_grid_near`).
static const float ROOM_BODY_REACH = 0.6;

// Mirrors `GpuRoomGrid` in crates/forge-render/src/probes/rooms.rs. Probe (i, j, k) stands at
// origin + i·step.x·axis + j·step.y·up + k·step.z·across, across the axis turned a quarter.
public struct RoomGrid
{
    public float3 origin; // its first probe's place, scene frame
    public uint first;    // its first probe among every probe
    public float2 axis;   // its x axis on the ground (x and z, unit)
    public float spacing; // its steps across the floor, the larger
    public uint region;   // its room
    public float3 step;   // metres between its probes along x, y (up) and across
    public float fade;    // its share of its room's light against the cascades', 0 to 1
    public uint3 counts;  // its probes along x, y and across, 2 at least each
    public uint next;     // the next grid of the same region, ROOM_NONE at the last
    public uint fresh;    // 1 on its first frame: nothing measured yet
    public uint pad0;
    public uint pad1;
    public uint pad2;
};

// Mirrors `GpuProbeField` in crates/forge-render/src/probes.rs: one per frame.
public struct ProbeField
{
    public float4 rotation[3];    // rows of this frame's random rotation of the rays
    public ProbeCascade cascades[PROBE_MAX_CASCADES];
    public uint atlas_row;        // probes a row of the atlases: the widest cascade's x × y
    public uint atlas_pad0;
    public uint atlas_pad1;
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
    public float settled_hysteresis; // the same for a settled probe's update: hysteresis^cadence
    public uint cadence;          // a settled probe updates every `cadence` frames (issue #103)
    public uint phase;            // this frame's turn, 0..cadence-1
    public uint* probe_region;    // per probe: the region of where it stands (D-068), REGION_NONE in a recess or a doorway
    public RegionVolume* region_volumes; // the rooms, recesses and doorways, each set's recesses and doorways first
    public uint2* region_cells;   // per cell of a set's grid: the first of its volumes' indices and their count
    public uint* region_indices;  // the volumes' indices, cell after cell
    public RegionSet* region_sets; // the sets (#338): a streamed cell's each
    public RegionTop* region_top; // the coarse grid over the sets, hashed (`top_hash`)
    public uint* region_top_list; // the sets each coarse cell lists
    public float* region_splits;  // per volume: a doorway shut by its door (#327), its leaf this share of the way across from `region`; 0 open
    public float region_cell;     // a set's cells' side, metres
    public float region_top_cell; // the coarse cells' side, metres
    public uint region_top_mask;  // the coarse table's entries less one
    public uint region_count;     // volumes: 0, no regions (every probe lights every surface)
    public float far_room_sky;    // the open sky's share a far room without its probes takes (#321, ProbeParams)
    public float normal_bias;     // a surface's lookup point off it: this share of its normal (`ProbeParams::sample_bias`)
    public float view_bias;       // and this share of the way to the viewer
    public uint region_pad;
    public uint* room_slots;      // per probe of the room grids' pool (from `room_first`): its grid, ROOM_NONE when unused (D-078)
    public RoomGrid* room_grids;  // the rooms' own grids this frame
    public uint2* room_table;     // the regions' hash: (region, its first grid), (0, ROOM_NONE) empty
    public uint room_first;       // the pool's first probe: the cascades' probes come before it
    public uint room_mask;        // the hash's entries less one
    public uint room_count;       // grids this frame: 0, the cascades light every room
    public uint room_rays;        // rays a room grid's probe traces a frame (its first PROBE_FIXED_RAYS fixed), at most `rays`
};

// The coarse cell `key`'s first entry in a table of `mask + 1` (`top_hash` in regions.rs).
uint top_hash(int2 key, uint mask)
{
    uint h = uint(key.x) * 0x8DA6B343u ^ uint(key.y) * 0xD8163841u;
    return (h ^ (h >> 15)) & mask;
}

// The sets the coarse cell holding `p` lists: the first in `region_top_list` and their count.
uint2 region_top_run(ProbeField* field, float2 p)
{
    int2 key = int2(floor(p / field[0].region_top_cell));
    uint mask = field[0].region_top_mask;
    uint at = top_hash(key, mask);
    // Linear probing in a table at most half full: an empty entry ends the run.
    for (uint k = 0u; k <= mask; k++)
    {
        RegionTop e = field[0].region_top[at];
        if (e.count == 0u)
            break;
        if (all(e.key == key))
            return uint2(e.first, e.count);
        at = (at + 1u) & mask;
    }
    return uint2(0u, 0u);
}

// The region at `p` (scene frame): the first volume holding it, in their order; outside them all,
// REGION_OUTSIDE.
public RegionHit region_at(ProbeField* field, float3 p)
{
    RegionHit hit;
    hit.region = REGION_OUTSIDE;
    hit.other = REGION_OUTSIDE;
    hit.ramp = 0.0;
    hit.kind = REGION_ROOM;
    if (field[0].region_count == 0u)
        return hit;
    // The sets the coarse cell lists, in the order they were added (#338), then each set's grid.
    uint2 run = region_top_run(field, p.xz);
    for (uint s = 0u; s < run.y; s++)
    {
        RegionSet rs = field[0].region_sets[field[0].region_top_list[run.x + s]];
        // The point relative to the set (its volumes are): a shift of the scene's origin moves
        // the set's alone.
        float3 q = p - rs.origin;
        int2 c = int2(floor(q.xz / field[0].region_cell));
        int2 dims = int2(rs.dims);
        if (any(c < 0) || any(c >= dims))
            continue;
        // The cell clamped into the grid before the read. Unclamped, the lookup made the physics
        // lab's frames differ from run to run (scattered pixels a few levels off), though with no
        // regions it returns above: the GPU read ahead of the return, far outside the grid. A
        // scene without regions names a buffer of zeros (`Probes::no_regions`), not null.
        c = clamp(c, int2(0), max(dims - 1, int2(0)));
        uint2 range = field[0].region_cells[rs.cells + uint(c.y * dims.x + c.x)];
        for (uint i = 0u; i < range.y; i++)
        {
            uint index = field[0].region_indices[range.x + i];
            RegionVolume v = field[0].region_volumes[index];
            if (q.y < v.floor || q.y > v.ceiling)
                continue;
            bool inside = true;
            for (uint k = 0u; k < 4u; k++)
            {
                float2 a = v.corners[k];
                float2 e = v.corners[(k + 1u) & 3u] - a;
                float2 d = q.xz - a;
                inside = inside && e.x * d.y - e.y * d.x >= 0.0;
            }
            if (!inside)
                continue;
            hit.region = v.region;
            hit.other = v.other;
            hit.kind = v.kind;
            if (v.kind == REGION_DOORWAY)
            {
                // Across the doorway from corners 0–1 to corners 3–2.
                float2 e = v.corners[1] - v.corners[0];
                float2 across = normalize(float2(-e.y, e.x));
                float depth = dot(v.corners[3] - v.corners[0], across);
                hit.ramp = saturate(dot(q.xz - v.corners[0], across) / max(depth, 1e-4));
                // Shut by its door (#327): each side of the leaf takes its own side's probes alone.
                float split = field[0].region_splits[index];
                if (split > 0.0)
                    hit.ramp = hit.ramp < split ? 0.0 : 1.0;
            }
            return hit;
        }
    }
    return hit;
}

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

// The direction of a probe's ray `ray` of `rays` this frame: the fixed rays are a lattice of their
// own; the others a lattice turned by the frame's rotation, so the probes see new directions every
// frame and the blend averages them.
public float3 probe_ray_direction(ProbeField* field, uint ray, uint rays)
{
    if (ray < PROBE_FIXED_RAYS)
        return fibonacci_direction(ray, PROBE_FIXED_RAYS);
    float3 d = fibonacci_direction(ray - PROBE_FIXED_RAYS, rays - PROBE_FIXED_RAYS);
    return float3(dot(field[0].rotation[0].xyz, d), dot(field[0].rotation[1].xyz, d), dot(field[0].rotation[2].xyz, d));
}

// ------------------------------------------------------------------- the grid ---

// Probes live in their cascade's slots by world cell modulo the counts, so a cascade that
// scrolls with the camera keeps every probe it still covers where it was; only the planes it
// enters are new (and take the slots of the planes it left).
public uint probe_index(ProbeField* field, uint cascade, int3 cell)
{
    ProbeCascade c = field[0].cascades[cascade];
    int3 n = int3(c.counts);
    int3 slot = wrap_cells(cell, n);
    return (c.first_row + uint(slot.z)) * field[0].atlas_row + uint(slot.y * n.x + slot.x);
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
    public int3 local;      // its slot in the cascade's volume
    public bool unused;     // a slot past a narrower cascade's x × y in its atlas row: no probe
    public uint grid;       // a room grid's probe (D-078): its grid; ROOM_NONE for a cascade's
};

public ProbeSlot probe_slot(ProbeField* field, uint probe)
{
    if (probe >= field[0].room_first)
        return room_probe_slot(field, probe);
    uint row = probe / field[0].atlas_row;
    uint along = probe % field[0].atlas_row;
    ProbeSlot s;
    s.cascade = 0u;
    s.grid = ROOM_NONE;
    for (uint k = 1u; k < field[0].cascade_count; k++)
    {
        if (row >= field[0].cascades[k].first_row)
            s.cascade = k;
    }
    ProbeCascade c = field[0].cascades[s.cascade];
    int3 n = int3(c.counts);
    s.unused = along >= uint(n.x * n.y);
    int3 slot = int3(int(along % uint(n.x)), int(along / uint(n.x)), int(row - c.first_row));
    s.local = slot;
    s.cell = c.origin + wrap_cells(slot - c.origin, n);
    s.spacing = c.spacing;
    s.position = (float3(s.cell) + 0.5) * c.spacing;
    s.fresh = any(s.cell < c.previous) || any(s.cell >= c.previous + n);
    return s;
}

// Probe (i, j, k) of room grid `g`, without its offset.
public float3 room_probe_position(RoomGrid g, int3 l)
{
    float3 axis = float3(g.axis.x, 0.0, g.axis.y);
    float3 across = float3(-g.axis.y, 0.0, g.axis.x);
    return g.origin + axis * (float(l.x) * g.step.x) + float3(0.0, float(l.y) * g.step.y, 0.0) + across * (float(l.z) * g.step.z);
}

// A probe of the room grids' pool (D-078): its grid's, or unused. Its slot in its grid is its
// `local` (and `cell`); its spacing the grid's across the floor.
// The rays probe `slot` traces a frame: a room grid's its own count (D-078), else the field's.
public uint probe_rays(ProbeField* field, ProbeSlot slot)
{
    return slot.grid != ROOM_NONE ? field[0].room_rays : field[0].rays;
}

ProbeSlot room_probe_slot(ProbeField* field, uint probe)
{
    ProbeSlot s;
    s.cascade = ROOM_CASCADE;
    s.grid = field[0].room_slots[probe - field[0].room_first];
    s.unused = s.grid == ROOM_NONE;
    s.cell = int3(0);
    s.local = int3(0);
    s.position = float3(0.0);
    s.spacing = 1.0;
    s.fresh = false;
    if (s.unused)
        return s;
    RoomGrid g = field[0].room_grids[s.grid];
    uint i = probe - g.first;
    int3 l = int3(int(i % g.counts.x), int((i / g.counts.x) % g.counts.y), int(i / (g.counts.x * g.counts.y)));
    s.local = l;
    s.cell = l;
    s.position = room_probe_position(g, l);
    s.spacing = g.spacing;
    s.fresh = g.fresh != 0u;
    return s;
}

// Whether probe `probe` takes this frame's update (issue #103): a new or young probe every
// frame, a settled one every `cadence` frames, on its turn. The turns follow the probe's slot
// (x + y + z), which a world cell keeps while its cascade scrolls, so neighbours take turns
// and each probe keeps its own. A room grid's probe updates every frame (D-078): the
// grids hold few probes, and nothing ranks them. `age` is the probe's before this frame's state pass; the blend,
// after it, passes `after_state` and reads the age that pass wrote (PROBE_SETTLE_UPDATES + 1
// for a probe settled before this update).
public bool probe_due(ProbeField* field, uint probe, ProbeSlot slot, float age, bool after_state)
{
    bool young = after_state ? age <= PROBE_SETTLE_UPDATES : age < PROBE_SETTLE_UPDATES;
    if (slot.fresh || young || field[0].cadence <= 1u || slot.grid != ROOM_NONE)
        return true;
    uint turn = uint(slot.local.x + slot.local.y + slot.local.z);
    return (turn + field[0].phase) % field[0].cadence == 0u;
}

// Where probe `probe`'s map of `texels` a side (its tile `tile` a side, border included)
// starts in its atlas: the atlas holds a row of x × y tiles per z of every cascade.
public uint2 probe_tile(ProbeField* field, uint probe, uint tile)
{
    uint row = field[0].atlas_row;
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
    return cascade_weight_over(field, c, p, float3(1.0));
}

// `cascade_weight` fading over `band` of the cascade's spacings along each axis instead of one.
float cascade_weight_over(ProbeField* field, uint c, float3 p, float3 band)
{
    float s = field[0].cascades[c].spacing;
    float3 reach = (float3(field[0].cascades[c].counts) * 0.5 - 1.5) * s;
    float3 w = saturate((reach - abs(p - field[0].camera)) / (band * s));
    return w.x * w.y * w.z;
}

// Whether no room-sized cascade (probes at most PROBE_ROOM_SPACING apart) covers `p`.
bool far_from_rooms(ProbeField* field, float3 p)
{
    for (uint c = 0; c < field[0].cascade_count; c++)
    {
        if (field[0].cascades[c].spacing <= PROBE_ROOM_SPACING && cascade_weight(field, c, p) > 0.0)
            return false;
    }
    return true;
}

// Irradiance at `p` (unit normal `n` for the light's direction, `g` the surface's own,
// interpolated normal, `v` towards the viewer) from cascade `c`'s eight probes around it, in the
// resolve's units (the cosine-weighted mean of the radiance the rays met, as the shading
// multiplies it by the albedo); w is 1 when at least one of them counts, else 0. With MIRROR,
// also the irradiance for a normal along `d` (`toward_d`), from the same probes with the same
// weights: one more fetch each (issue #68). With `masked`, only the probes of `mask`'s region
// count (D-068): across a doorway, those of its two sides, each as far as the point stands
// towards it.
public float4 cascade_irradiance<let MIRROR : int>(ProbeField* field, uint c, float3 p, float3 n, float3 g, float3 v, float3 d, RegionHit mask, bool masked, out float3 toward_d)
{
    toward_d = float3(0.0);
    ProbeCascade cascade = field[0].cascades[c];
    float s = cascade.spacing;
    int3 counts = int3(cascade.counts);
    // The 2021 paper's bias: off the surface, mostly towards the viewer (`ProbeParams::sample_bias`,
    // 0.2 of the normal and 0.8 of the view by default), in proportion to the spacing, so the visibility test does not sit on the surface's own shadow boundary. Along
    // the surface's interpolated normal, not the bent one of its normal map (D-068; EA's GIBS
    // takes the vertices' normal): a bump's normal leaned the point into the wall.
    float3 q = p + (g * field[0].normal_bias + v * field[0].view_bias) * (0.75 * 0.3 * s);
    float3 grid = q / s - 0.5;
    int3 base = int3(floor(grid));
    float3 alpha = grid - float3(base);
    int3 clamped = clamp(base, cascade.origin, cascade.origin + counts - 2);
    alpha = saturate(alpha + float3(base - clamped));
    base = clamped;
    // The corners' slots step from the base's, wrapping by a subtraction: `probe_index` and
    // `probe_tile` divide, and 8 probes' divisions cost the resolve more than their fetches.
    int3 base_slot = wrap_cells(base, counts);
    uint row = field[0].atlas_row;
    float2 texel_n = (oct_encode(n) * 0.5 + 0.5) * float(IRRADIANCE_TEXELS);
    float2 texel_d = (oct_encode(d) * 0.5 + 0.5) * float(IRRADIANCE_TEXELS);
    float3 sum = float3(0.0);
    float3 sum_d = float3(0.0);
    float total = 0.0;
    bool found = false;
    // A room's lookup (masked, not the outside's): how much of the trilinear weight its own probes
    // hold, before visibility (`PROBE_ROOM_COVER`).
    bool room_mask = masked && mask.region != REGION_OUTSIDE;
    float coverage = 0.0;
    [unroll]
    for (uint k = 0; k < 8; k++)
    {
        int3 corner = int3(int(k & 1u), int((k >> 1) & 1u), int(k >> 2));
        // A probe whose trilinear weight is negligible is skipped before its fetches (#70), and one
        // just over it fades in, so its light does not switch on along a line. In front of the
        // surface the threshold falls to PROBE_LEAST_FRONT: in a room's corner the probes in front
        // of a wall may be the room's only ones to see it, at a few percent each, and without
        // them the light of those behind it, outside, came in round a window's head
        // (2026-10-17). The threshold falls smoothly over a quarter of a cell in front of the
        // surface's plane, so no line shows where a probe's cell crosses it. Masked by region,
        // the region's probes count down to PROBE_LEAST_FRONT, the others not at all.
        float3 tri = max(lerp(1.0 - alpha, alpha, float3(corner)), 0.001);
        float trilinear = tri.x * tri.y * tri.z;
        int3 cell = base + corner;
        int3 slot = base_slot + corner;
        slot = select(slot >= counts, slot - counts, slot);
        uint2 tile = uint2(uint(slot.y * counts.x + slot.x), cascade.first_row + uint(slot.z));
        uint probe = tile.y * row + tile.x;
        float share = 1.0;
        float keep;
        if (masked)
        {
            uint region = field[0].probe_region[probe];
            share = region == mask.region ? 1.0 - mask.ramp : (region == mask.other ? mask.ramp : 0.0);
            keep = saturate(trilinear / PROBE_LEAST_FRONT - 1.0);
            if (share <= 0.0 || keep <= 0.0)
                continue;
        }
        else
        {
            // Coarser cascades compute #70's own expression, so their light stays the same to the bit.
            bool room = s <= PROBE_ROOM_SPACING;
            float least = lerp(PROBE_LEAST_WEIGHT, PROBE_LEAST_FRONT, saturate(4.0 * dot((float3(cell) + 0.5) * s - p, n) / s));
            keep = room ? saturate(trilinear / least - 1.0) : saturate(trilinear / PROBE_LEAST_WEIGHT - 1.0);
            if (room ? keep <= 0.0 : trilinear < PROBE_LEAST_WEIGHT)
                continue;
        }
        float4 data = field[0].probe_data[probe];
        if (!probe_lights(data.w))
            continue;
        found = true;
        coverage += trilinear * keep * share;
        float3 position = (float3(cell) + 0.5) * s + data.xyz;
        // Probes behind the surface count less, smoothly.
        float3 to_probe = normalize(position - p);
        float facing = (dot(to_probe, g) + 1.0) * 0.5;
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
            float gap = r - moments.x;
            float chebyshev = variance / (variance + gap * gap);
            w *= max(0.05, chebyshev * chebyshev * chebyshev);
        }
        w = max(w, 1e-6);
        // Weights that are already small shrink further: a faint leak reads as light in the dark.
        if (w < 0.2)
            w *= w * w / (0.2 * 0.2);
        w *= trilinear * keep * share;
        float2 corner_texel = float2(tile * IRRADIANCE_TILE);
        float3 e = sample_image_level(field[0].irradiance_image, SAMPLER_LINEAR_CLAMP,
            (corner_texel + 1.0 + texel_n) * field[0].irradiance_texel, 0.0).rgb;
        // Blend the square roots: a leak from a bright probe weighs less.
        sum += w * sqrt(max(e, 0.0));
        if (MIRROR != 0)
        {
            float3 e_d = sample_image_level(field[0].irradiance_image, SAMPLER_LINEAR_CLAMP,
                (corner_texel + 1.0 + texel_d) * field[0].irradiance_texel, 0.0).rgb;
            sum_d += w * sqrt(max(e_d, 0.0));
        }
        total += w;
    }
    if (!found || total <= 0.0)
        return float4(0.0);
    float3 root = sum / total;
    if (MIRROR != 0)
    {
        float3 root_d = sum_d / total;
        toward_d = root_d * root_d;
    }
    return float4(root * root, room_mask ? saturate(coverage / PROBE_ROOM_COVER) : 1.0);
}

// The cascades' light at `p`, finer first, each over what the finer ones left, and how much of
// it they covered (w).
float4 cascades_lookup<let MIRROR : int>(ProbeField* field, float3 p, float3 n, float3 g, float3 v, float3 d, RegionHit mask, bool masked, bool rooms, out float3 toward_d)
{
    float3 e = float3(0.0);
    float3 e_d = float3(0.0);
    float covered = 0.0;
    for (uint c = 0; c < field[0].cascade_count && covered < 0.999; c++)
    {
        if (!rooms && field[0].cascades[c].spacing <= PROBE_ROOM_SPACING)
            continue;
        // A room's surface leaves a room-sized cascade for the next over ROOM_BLEND of its
        // spacings (#353): where their light differs, a far wall changed within a metre.
        bool room_blend = masked && mask.region != REGION_OUTSIDE && field[0].cascades[c].spacing <= PROBE_ROOM_SPACING;
        float w = room_blend ? cascade_weight_over(field, c, p, ROOM_BAND) : cascade_weight(field, c, p);
        if (w <= 0.0)
            continue;
        float3 ce_d;
        float4 ce = cascade_irradiance<MIRROR>(field, c, p, n, g, v, d, mask, masked, ce_d);
        if (ce.w <= 0.0)
            continue;
        float share = (1.0 - covered) * w * ce.w;
        e += share * ce.rgb;
        e_d += share * ce_d;
        covered += share;
    }
    toward_d = e_d;
    return float4(e, covered);
}

// The cascades' light at a surface (`probe_lookup`) for its region `mask` (`masked` when regions
// apply), and with MIRROR the same for a normal along `d` (`toward_d`, over `open_sky_d`). With
// regions (D-068), the surface's is where it faces, 2 cm off it along `g`;
// where no cascade has a probe of it among the eight around the point, every probe counts, as
// without regions (Unity's rendering layers fall back so). A surface in a house's body but in
// none of its rooms (a face whose point 2 cm off it missed its room's outline: a corner, the
// edge of a ceiling) is lit by every probe, as without regions: taken for the outside's, such
// pixels inside took the sunlit probes' light, white specks down a corner.
float3 probe_lookup_cascades<let MIRROR : int>(ProbeField* field, float3 p, float3 n, float3 g, float3 v, float3 open_sky, float3 d, float3 open_sky_d, RegionHit mask, bool masked, out float3 toward_d)
{
    bool unused_far;
    float3 near = probe_lookup_with<MIRROR>(field, p, n, g, v, open_sky, d, open_sky_d, mask, masked, true, toward_d, unused_far);
    if (!masked || mask.region == REGION_OUTSIDE)
        return near;
    // A room near the room-sized cascades' edge, with none of its probes in the coarser ones
    // (#334): their light blended into the far room's share of the open sky it takes beyond
    // them (#321) over ROOM_BLEND spacings across the ground, not switched over the cascade's
    // own one. A window seen while walking towards its building popped from dark to lit within
    // a metre. Where a coarser cascade has the room's probes, its light takes over across the
    // same band (`cascades_lookup`, #353).
    float k = room_depth(field, p);
    if (k <= 0.0 || k >= 1.0)
        return near;
    float3 beyond_d;
    bool far;
    float3 beyond = probe_lookup_with<MIRROR>(field, p, n, g, v, open_sky, d, open_sky_d, mask, masked, false, beyond_d, far);
    if (!far)
        return near;
    toward_d = lerp_exact(beyond_d, toward_d, k);
    return lerp_exact(beyond, near, k);
}

// ------------------------------------------------------------- room grids' lookup ---

// The entry of `region` in the rooms' hash of `mask + 1` (`room_hash` in rooms.rs).
uint room_hash(uint region, uint mask)
{
    uint h = region;
    h ^= h >> 16;
    h *= 0x7FEB352Du;
    h ^= h >> 15;
    h *= 0x846CA68Bu;
    h ^= h >> 16;
    return h & mask;
}

// `p` in room grid `g`'s probe steps: (0, 0, 0) at its first probe, counts − 1 at its last.
float3 room_local(RoomGrid g, float3 p)
{
    float3 r = p - g.origin;
    return float3(r.x * g.axis.x + r.z * g.axis.y, r.y, r.z * g.axis.x - r.x * g.axis.y) / g.step;
}

// The grid of room `region` for `p` (D-078): the one whose box holds it, else the nearest of the
// room's grids (a room of several volumes has one per volume); ROOM_NONE where the room has none.
public uint room_grid_of(ProbeField* field, uint region, float3 p)
{
    if (field[0].room_count == 0u || region == REGION_OUTSIDE)
        return ROOM_NONE;
    uint mask = field[0].room_mask;
    uint at = room_hash(region, mask);
    uint first = ROOM_NONE;
    // Linear probing in a table at most half full: an empty entry ends the run.
    for (uint k = 0u; k <= mask; k++)
    {
        uint2 e = field[0].room_table[at];
        if (e.y == ROOM_NONE)
            break;
        if (e.x == region)
        {
            first = e.y;
            break;
        }
        at = (at + 1u) & mask;
    }
    if (first == ROOM_NONE || field[0].room_grids[first].next == ROOM_NONE)
        return first;
    uint best = first;
    float nearest = 1e30;
    uint g = first;
    for (uint k = 0u; k < 16u && g != ROOM_NONE; k++)
    {
        RoomGrid grid = field[0].room_grids[g];
        float3 local = room_local(grid, p);
        float3 beyond = max(max(-local, local - float3(grid.counts - 1u)), 0.0) * grid.step;
        float d = dot(beyond, beyond);
        if (d < nearest)
        {
            nearest = d;
            best = g;
        }
        g = grid.next;
    }
    return best;
}

// The grid whose room stands nearest `p`, within ROOM_BODY_REACH of its box (its probes' and the
// inset), for a point in a house's body (REGION_SOLID) whose own region tells nothing: a face
// whose point 2 cm off it missed its room's outline (a corner), a window's frame, a slab's
// edge. ROOM_NONE farther from them all. Those points took every probe's light; with their
// rooms' cascade probes retired, the outside's alone, and the grids' probes that met them lit
// their rooms 6 levels too bright.
uint room_grid_near(ProbeField* field, float3 p)
{
    uint best = ROOM_NONE;
    float nearest = ROOM_BODY_REACH * ROOM_BODY_REACH;
    for (uint g = 0u; g < field[0].room_count; g++)
    {
        RoomGrid grid = field[0].room_grids[g];
        float3 local = room_local(grid, p);
        float3 beyond = max(max(-local, local - float3(grid.counts - 1u)), 0.0) * grid.step;
        beyond = max(beyond - ROOM_INSET, 0.0);
        float d = dot(beyond, beyond);
        if (d <= nearest)
        {
            nearest = d;
            best = g;
        }
    }
    return best;
}

// Irradiance at `p` from room grid `index`'s eight probes around it (D-078), weighed as
// `cascade_irradiance` weighs a cascade's (facing, Chebyshev, the crush) but blended linearly:
// the square roots' blend darkened rooms by its spread (research §2.2). A point beyond the grid's
// outer probes (a window's reveal, the last 0.35 m before a wall) takes their light. w is 1 when
// one of them counts, else 0.
float4 room_irradiance<let MIRROR : int>(ProbeField* field, uint index, float3 p, float3 n, float3 g, float3 v, float3 d, out float3 toward_d)
{
    toward_d = float3(0.0);
    RoomGrid grid = field[0].room_grids[index];
    float s = grid.spacing;
    float3 q = p + (g * field[0].normal_bias + v * field[0].view_bias) * (0.75 * 0.3 * s);
    float3 local = room_local(grid, q);
    int3 counts = int3(grid.counts);
    int3 base = clamp(int3(floor(local)), int3(0), counts - 2);
    float3 alpha = saturate(local - float3(base));
    uint row = field[0].atlas_row;
    // The base probe's tile, and the corners' stepped from it: 8 probes' divisions cost the resolve
    // more than their fetches (as in `cascade_irradiance`).
    uint base_probe = grid.first + uint((base.z * counts.y + base.y) * counts.x + base.x);
    uint2 base_tile = uint2(base_probe % row, base_probe / row);
    float2 texel_n = (oct_encode(n) * 0.5 + 0.5) * float(IRRADIANCE_TEXELS);
    float2 texel_d = (oct_encode(d) * 0.5 + 0.5) * float(IRRADIANCE_TEXELS);
    float3 sum = float3(0.0);
    float3 sum_d = float3(0.0);
    float total = 0.0;
    bool found = false;
    [loop]
    for (uint k = 0; k < 8; k++)
    {
        int3 corner = int3(int(k & 1u), int((k >> 1) & 1u), int(k >> 2));
        float3 tri = max(lerp(1.0 - alpha, alpha, float3(corner)), 0.001);
        float trilinear = tri.x * tri.y * tri.z;
        int3 l = base + corner;
        // A probe of negligible trilinear weight is skipped before its fetches, fading in over
        // twice it (PROBE_LEAST_FRONT, as a room's cascade lookup does).
        float keep = saturate(trilinear / PROBE_LEAST_FRONT - 1.0);
        if (keep <= 0.0)
            continue;
        uint probe = grid.first + uint((l.z * counts.y + l.y) * counts.x + l.x);
        float4 data = field[0].probe_data[probe];
        if (!probe_lights(data.w))
            continue;
        found = true;
        float3 position = room_probe_position(grid, l) + data.xyz;
        float3 to_probe = normalize(position - p);
        float facing = (dot(to_probe, g) + 1.0) * 0.5;
        float w = facing * facing + 0.2;
        float3 out_dir = q - position;
        float r = max(length(out_dir), 1e-4);
        uint along = base_tile.x + uint((corner.z * counts.y + corner.y) * counts.x + corner.x);
        uint2 tile = uint2(along, base_tile.y);
        while (tile.x >= row)
        {
            tile.x -= row;
            tile.y++;
        }
        float2 moments = sample_image_level(field[0].distance_image, SAMPLER_LINEAR_CLAMP,
            (float2(tile * DISTANCE_TILE) + 1.0 + (oct_encode(out_dir / r) * 0.5 + 0.5) * float(DISTANCE_TEXELS)) * field[0].distance_texel, 0.0).xy;
        if (r > moments.x)
        {
            float variance = abs(moments.y - moments.x * moments.x);
            float gap = r - moments.x;
            float chebyshev = variance / (variance + gap * gap);
            w *= max(0.05, chebyshev * chebyshev * chebyshev);
        }
        w = max(w, 1e-6);
        if (w < 0.2)
            w *= w * w / (0.2 * 0.2);
        w *= trilinear * keep;
        float2 corner_texel = float2(tile * IRRADIANCE_TILE);
        float3 e = sample_image_level(field[0].irradiance_image, SAMPLER_LINEAR_CLAMP,
            (corner_texel + 1.0 + texel_n) * field[0].irradiance_texel, 0.0).rgb;
        sum += w * max(e, 0.0);
        if (MIRROR != 0)
        {
            float3 e_d = sample_image_level(field[0].irradiance_image, SAMPLER_LINEAR_CLAMP,
                (corner_texel + 1.0 + texel_d) * field[0].irradiance_texel, 0.0).rgb;
            sum_d += w * max(e_d, 0.0);
        }
        total += w;
    }
    if (!found || total <= 0.0)
        return float4(0.0);
    if (MIRROR != 0)
        toward_d = sum_d / total;
    return float4(sum / total, 1.0);
}

// What the room grids give a surface or a hit at `p` (D-078): their light, each side's grid at its
// fade (a doorway's two sides weighed by its ramp), and how much of the point's light that is (w).
// `rest` is the region the cascades light the remainder for: the point's own `mask`, or the one
// side no grid covers whole; `rest_masked` whether regions apply to it.
struct RoomShare
{
    float3 e;
    float3 e_d;
    float w;
    RegionHit rest;
    bool rest_masked;
};

// A region of its own, for the cascades' light on one side alone.
RegionHit region_alone(uint region)
{
    RegionHit h;
    h.region = region;
    h.other = region;
    h.ramp = 0.0;
    h.kind = REGION_ROOM;
    return h;
}

// The grids' share at `p` (`RoomShare`), `mask` and `masked` the point's region as the lookup found
// it, `v` the view for the grids' bias. With MIRROR, also their light for a normal along `d`.
// Each grid is read once, in a loop over the sides, so the driver compiles the lookup once: every
// call site of a lookup is a whole inlined copy (#348; a copy per side and branch made the resolve
// a 220 s pipeline).
RoomShare room_share<let MIRROR : int>(ProbeField* field, float3 p, float3 n, float3 g, float3 v, float3 d, RegionHit mask, bool masked)
{
    RoomShare r;
    r.e = float3(0.0);
    r.e_d = float3(0.0);
    r.w = 0.0;
    r.rest = mask;
    r.rest_masked = masked;
    if (field[0].room_count == 0u || field[0].region_count == 0u)
        return r;
    // The sides: (region, grid, share).
    uint region0 = mask.region;
    uint region1 = mask.other;
    uint grid0 = ROOM_NONE;
    uint grid1 = ROOM_NONE;
    float share0 = 1.0;
    float share1 = 0.0;
    if (mask.kind == REGION_SOLID)
    {
        // A house's body: the grid within reach, its room the side (`room_grid_near`).
        grid0 = room_grid_near(field, p);
        if (grid0 == ROOM_NONE)
            return r;
        region0 = field[0].room_grids[grid0].region;
        region1 = region0;
    }
    else if (masked)
    {
        // (A doorway's first side may be the outside, its second a room: `room_grid_of` tells.)
        bool doorway = mask.kind == REGION_DOORWAY;
        grid0 = mask.ramp < 1.0 ? room_grid_of(field, mask.region, p) : ROOM_NONE;
        grid1 = doorway && mask.ramp > 0.0 ? room_grid_of(field, mask.other, p) : ROOM_NONE;
        if (doorway)
        {
            share0 = 1.0 - mask.ramp;
            share1 = mask.ramp;
        }
    }
    if (grid0 == ROOM_NONE && grid1 == ROOM_NONE)
        return r;
    float covered0 = 0.0;
    float covered1 = 0.0;
    [loop]
    for (uint k = 0u; k < 2u; k++)
    {
        uint grid = k == 0u ? grid0 : grid1;
        float share = k == 0u ? share0 : share1;
        if (grid == ROOM_NONE || share <= 0.0)
            continue;
        float fade = field[0].room_grids[grid].fade;
        if (fade <= 0.0)
            continue;
        float3 e_d;
        float4 e = room_irradiance<MIRROR>(field, grid, p, n, g, v, d, e_d);
        if (e.w <= 0.0)
            continue; // none of its probes lights anything here: the cascades
        r.e += (share * fade) * e.rgb;
        r.e_d += (share * fade) * e_d;
        r.w += share * fade;
        if (k == 0u)
            covered0 = fade;
        else
            covered1 = fade;
    }
    // The cascades light the rest: a house's body point its grid's room alone, even where none of the
    // grid's probes lit it (a stair's mass: every probe's light there is the outside's once the
    // room's cascade probes retire, and the stairwell came out 2 to 3 levels too bright); one side
    // alone where the other's grid covers it whole; else the point's region as it stands (both sides
    // fading at a doorway: their cascades blended by its ramp, as without grids).
    if (mask.kind == REGION_SOLID)
    {
        r.rest = region_alone(region0);
        r.rest_masked = true;
    }
    else if (covered0 >= 1.0 && share1 > 0.0)
    {
        r.rest = region_alone(region1);
        r.rest_masked = true;
    }
    else if (covered1 >= 1.0 && share0 > 0.0)
    {
        r.rest = region_alone(region0);
        r.rest_masked = true;
    }
    return r;
}

// `probe_irradiance` and its mirror twin: a room with a grid of its own takes its light from it
// (D-078, `room_share`), every other surface, and what the grids leave, from the cascades
// (`probe_lookup_cascades`, called once).
float3 probe_lookup<let MIRROR : int>(ProbeField* field, float3 p, float3 n, float3 g, float3 v, float3 open_sky, float3 d, float3 open_sky_d, out float3 toward_d)
{
    RegionHit mask = region_at(field, p + g * 0.02);
    bool masked = field[0].region_count > 0u && mask.kind != REGION_SOLID;
    RoomShare rooms = room_share<MIRROR>(field, p, n, g, v, d, mask, masked);
    if (rooms.w >= 1.0)
    {
        toward_d = rooms.e_d;
        return rooms.e;
    }
    float3 c_d;
    float3 c = probe_lookup_cascades<MIRROR>(field, p, n, g, v, open_sky, d, open_sky_d, rooms.rest, rooms.rest_masked, c_d);
    if (rooms.w <= 0.0)
    {
        toward_d = c_d;
        return c;
    }
    toward_d = rooms.e_d + (1.0 - rooms.w) * c_d;
    return rooms.e + (1.0 - rooms.w) * c;
}

// How deep `p` stands in the room-sized cascades across the ground (#334): 1 from ROOM_BLEND
// of their spacings inside their edge along x and z (one spacing up and down, where they reach
// a few metres), falling to 0 at it and beyond; 1 where there are none.
float room_depth(ProbeField* field, float3 p)
{
    float k = 0.0;
    bool any = false;
    for (uint c = 0; c < field[0].cascade_count; c++)
    {
        if (field[0].cascades[c].spacing > PROBE_ROOM_SPACING)
            continue;
        any = true;
        k = max(k, cascade_weight_over(field, c, p, ROOM_BAND));
    }
    return any ? k : 1.0;
}

// `probe_lookup`'s light for a region `mask` (`masked` when regions apply), with the room-sized
// cascades (`rooms`) or as if the point stood beyond them; `far`: it took a far room's share of
// the open sky (#321).
float3 probe_lookup_with<let MIRROR : int>(ProbeField* field, float3 p, float3 n, float3 g, float3 v, float3 open_sky, float3 d, float3 open_sky_d, RegionHit mask, bool masked, bool rooms, out float3 toward_d, out bool far)
{
    far = false;
    // The region's probes, then every probe where they found none.
    float4 e = cascades_lookup<MIRROR>(field, p, n, g, v, d, mask, masked, rooms, toward_d);
    if (masked && e.w <= 0.0 && mask.region != REGION_OUTSIDE && (!rooms || far_from_rooms(field, p)))
    {
        far = true;
        // A room beyond the room-sized cascades with none of its probes among a coarser one's
        // eight (an upper storey between probes 4 m apart, #321): the probes there are other
        // rooms', the street's behind a wall or the roof void's (which no region covers, so it
        // counts as outside), and the room came out black. It takes `far_room_sky` of the
        // open sky's light instead, what its windows let in, in the warm white the rooms' walls
        // give it back: the sky's own blue turned their floors blue.
        float3 tint = float3(1.10, 0.99, 0.80);
        float3 luma = float3(0.2126, 0.7152, 0.0722);
        e = float4(tint * (dot(open_sky, luma) * field[0].far_room_sky), 1.0);
        toward_d = tint * (dot(open_sky_d, luma) * field[0].far_room_sky);
    }
    if (masked && e.w <= 0.0)
        e = cascades_lookup<MIRROR>(field, p, n, g, v, d, mask, false, rooms, toward_d);
    else if (masked && mask.region != REGION_OUTSIDE && e.w < 1.0)
    {
        // A room's cascades covered part of it: theirs is all its light, not the open sky's.
        toward_d /= e.w;
        e = float4(e.rgb / e.w, 1.0);
    }
    toward_d += (1.0 - e.w) * open_sky_d;
    return e.rgb + (1.0 - e.w) * open_sky;
}

// Irradiance at `p` from the finest cascades that cover it, finer first, each over what the
// finer ones left; `open_sky` (the sky's irradiance at `n`, unoccluded) beyond them all. `n` is
// the normal the light is gathered for (a normal map's), `g` the surface's interpolated one.
public float3 probe_irradiance(ProbeField* field, float3 p, float3 n, float3 g, float3 v, float3 open_sky)
{
    float3 unused;
    return probe_lookup<0>(field, p, n, g, v, open_sky, n, open_sky, unused);
}

// `probe_irradiance` at a ray's hit (the probes' own rays, a mirror ray's), `n` its normal: the
// hit's region's probes alone, without the fallback to every probe, so the probes' rays carry one
// copy of the lookup (a second cost them 0.11 ms in a room). Where none of the region's probes
// stands among the eight, a hit in a house takes no bounce, rather than the sky's, and the
// outside's the open sky. `outside`: the caller knows the hit is the outside's (a probe's ray
// from a probe outside every house), and the lookup skips finding its region. A room with a grid
// of its own (D-078) bounces its light in its grid (`room_share`).
//
// In a room the hit's point is moved off it along its normal alone, not also towards where the
// ray came from (#353). That second offset grows with the spacing (0.36 m between probes 2 m
// apart), and through the probes' bounce it darkened the coarser cascades' rooms. In a village
// room lit by the 2 m cascade alone, it took 4–8 sRGB levels from the walls, floor and table.
// In the same room lit by the 1 m one it took under one. So a far wall brightened as the finest
// cascade reached it.
public float3 probe_irradiance_hit(ProbeField* field, float3 p, float3 n, float3 v, float3 open_sky, bool outside)
{
    RegionHit mask;
    mask.region = REGION_OUTSIDE;
    mask.other = REGION_OUTSIDE;
    mask.ramp = 0.0;
    mask.kind = REGION_ROOM;
    if (!outside)
        mask = region_at(field, p + n * 0.02);
    bool masked = field[0].region_count > 0u && mask.kind != REGION_SOLID;
    float3 unused;
    RoomShare rooms = room_share<0>(field, p, n, n, float3(0.0), n, mask, masked);
    if (rooms.w >= 1.0)
        return rooms.e;
    bool room = rooms.rest_masked && rooms.rest.region != REGION_OUTSIDE;
    float4 e = cascades_lookup<0>(field, p, n, n, room ? float3(0.0) : v, n, rooms.rest, rooms.rest_masked, true, unused);
    float3 c = room ? (e.w > 0.0 ? e.rgb / e.w : float3(0.0)) : e.rgb + (1.0 - e.w) * open_sky;
    if (rooms.w <= 0.0)
        return c;
    return rooms.e + (1.0 - rooms.w) * c;
}

// `probe_irradiance`, and the irradiance the probes find for a normal along the mirror
// direction `r` (`toward_r`, over the sky's unoccluded `open_sky_r` there), from the same
// probes: what dims the sky's reflection in a street (issue #68).
public float3 probe_irradiance_mirror(ProbeField* field, float3 p, float3 n, float3 g, float3 v, float3 open_sky, float3 r, float3 open_sky_r, out float3 toward_r)
{
    return probe_lookup<1>(field, p, n, g, v, open_sky, r, open_sky_r, toward_r);
}

// A region's colour in `probe_region_tint`: black outside, else one of its own.
float3 region_colour(uint region)
{
    if (region == REGION_OUTSIDE)
        return float3(0.0);
    uint h = region * 0x9E3779B9u;
    h ^= h >> 15;
    h *= 0x85EBCA6Bu;
    h ^= h >> 13;
    return 0.2 + 0.6 * float3(float(h & 255u), float((h >> 8) & 255u), float((h >> 16) & 255u)) / 255.0;
}

// The `--show-regions` view (D-068): the region whose probes light the surface at `p` (its
// interpolated normal `g`, `v` towards the viewer), as `probe_lookup` decides it. A room's own
// colour, a doorway's from one side's to the other's, green in a window's reveal, white in a
// house's body (every probe), black outside, red where no probe of the region stood among any
// cascade's eight and every probe lit it (the fallback), orange where a room beyond the
// room-sized cascades took the open sky's share instead (#321); grey without regions.
public float3 probe_region_tint(ProbeField* field, float3 p, float3 g, float3 v)
{
    if (field[0].region_count == 0u)
        return float3(0.5);
    RegionHit mask = region_at(field, p + g * 0.02);
    if (mask.kind == REGION_SOLID)
        return float3(1.0);
    float3 unused;
    if (cascades_lookup<0>(field, p, g, g, v, g, mask, true, true, unused).w <= 0.0)
        return mask.region != REGION_OUTSIDE && far_from_rooms(field, p) ? float3(1.0, 0.5, 0.0) : float3(1.0, 0.0, 0.0);
    if (mask.kind == REGION_RECESS)
        return float3(0.0, 1.0, 0.0);
    if (mask.kind == REGION_DOORWAY)
        return lerp(region_colour(mask.region), region_colour(mask.other), mask.ramp);
    return region_colour(mask.region);
}
