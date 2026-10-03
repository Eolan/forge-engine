// The lab's liquid (#156, D-044): particles carry the water, a grid keeps it incompressible.
// Driven by crates/forge-render/src/liquid.rs; drawn by liquid_draw.slang.
//
// APIC (Jiang et al. 2015) on a MAC grid: each particle carries its velocity and, per component,
// the affine vector c (the velocity's gradient), handed to the grid's faces with trilinear
// weights (`liquid/p2g`) and gathered back (`liquid/g2p`). The grid's sums are fixed-point
// atomics (a face's weight and momentum packed in one 64-bit add), so their order does not
// change them and a run replays to the same bits on one GPU (D-044). Each substep:
// - p2g: every particle's momentum and weight to the 2×2×2 faces of each component round it,
//   its count to its cell and its weight to the 2×2×2 cell centres round it (the density);
// - faces: the faces' velocities (momentum over weight), gravity, the glass's and the gate's;
// - cells: each cell air, liquid or solid; a liquid cell's outflow, and how crowded its particles
//   are against a full cell's (the drift correction, Kugelstadt et al. 2019's idea after Ten
//   Minute Physics);
// - pressure: red-black Gauss–Seidel with over-relaxation, warm-started from the last substep
//   (and from the hydrostatic pressure at the start), so the still water's pressure is there
//   from the first substep and the iterations only chase what changed;
// - project: the pressure's gradient taken from the faces between liquid cells and air;
// - g2p: each particle's velocity and c from the faces, then moved, nudged down the crowding's
//   gradient (a share of it undone a substep: a move, never a velocity, so it adds no energy;
//   as a target of the pressure's divergence, it did, and still water shook itself apart in
//   half a second), and kept inside the tank and out of the gate.
// Pressure here is in velocity units: a face's velocity loses the pressure's difference across
// it, so the still water's rises by g·dt a cell downwards.
//
// White water (D-044's answer 5, "the hole's jet should foam"; then the owner: "foam on clear, non
// salt water does not make too much sense"): each particle carries how much air it holds (its
// position's w, 0 to 1), taken in where fast water meets the air (its skin tears) and where it is
// stopped hard (an impact beats air in), and lost as the bubbles rise and burst over the
// liquid's `foam_life`: a fraction of a second in fresh water, which only whitens where the jet
// plunges; longer in salt water or with surfactants, where foam lasts; 0 for none.
// `liquid/foam` splats it to the cells once a frame for the drawing.

static const float FIX = 1048576.0; // fixed-point units of the grid's sums (2^20)
static const uint AIR = 0u;
static const uint LIQUID = 1u;
static const uint SOLID = 2u;
static const uint MAX_SUBSTEPS = 8u;
static const uint MG_LEVELS = 8u;
// The statistics' words (LIQUID_STAT_* in liquid.rs).
static const uint STAT_PARTICLES = 0u;
static const uint STAT_LOST = 1u;
static const uint STAT_HEIGHT_MM = 2u;
static const uint STAT_FRONT = 3u;
static const uint STAT_ENERGY = 4u;
static const uint STAT_DIGEST = 5u;
static const uint STAT_CELLS = 7u;
static const uint STAT_LEVEL = 8u;
static const uint STAT_COLUMNS = 9u;
static const uint STAT_VOLUME = 10u;
static const uint STAT_SPEED = 11u;
static const uint STAT_BEHIND = 12u;
static const uint STAT_RESIDUAL = 13u;
static const uint STAT_DIVERGENCE = 14u;
static const uint STAT_RESIDUAL_MAX = 15u;
// Foam: what the skin takes in a second per m/s over FOAM_SPEED where the water meets the air;
// what an impact beats in a second per m/s² over FOAM_IMPACT.
static const float FOAM_SKIN = 1.5;
static const float FOAM_SPEED = 1.5;
static const float FOAM_HARD = 0.02;
static const float FOAM_IMPACT = 60.0;

// Must match GpuLiquidFrame in liquid.rs.
struct LiquidFrame
{
    float4 size;      // xyz: the tank's inside, metres; w: a cell's side, metres
    uint4 cells;      // xyz: cells along x, y and z; w: particles
    uint4 block;      // xyz: the water's block at the start, cells from the tank's corner; w: unused
    float4 gate;      // x, y: the gate's faces along x, metres (y <= x: no gate); zw: unused
    float4 hole;      // xy: the hole's centre (its y and z), metres; z: its radius (0: none); w: unused
    float4 steps[MAX_SUBSTEPS]; // per substep: x: the gate's bottom, metres; y: its speed up, m/s;
                                // z: the shutter's bottom (the hole open below it), metres; w: unused
    float4 gravity;   // xyz: m/s² (the lab's 9.81 down by default; `--liquid-gravity`); w: unused
    float dt;         // a substep, seconds
    float rest;       // a full cell's density at rest (its particles' weight)
    float drift;      // the share of a cell's crowding the drift correction undoes a substep
    float omega;      // the pressure's over-relaxation
    uint nodes;       // (cells.x + 1)(cells.y + 1)(cells.z + 1): each component's faces
    uint cell_count;
    float foam_life;  // how long the air it takes in lasts, seconds (0: none)
    uint pad1;
    float4* position; // per particle: xyz, metres from the tank's inside corner; w: its foam
    float4* velocity; // xyz, m/s
    float4* affine;   // three per particle: c of the faces' x, y and z components, 1/s
    int64_t* sums;    // 3 × nodes: weight << 32 + momentum, both fixed-point
    float* face;      // 3 × nodes: the faces' velocities, m/s
    uint* count;      // per cell: the particles in it
    int* density;     // per cell: the particles' weight at its centre, fixed-point
    uint* kind;       // per cell: AIR, LIQUID or SOLID
    float* pressure;  // per cell, m/s
    float* divergence;// per cell: the outflow, m/s
    float* crowding;  // per liquid cell: its density over a full cell's, less 1 (0 elsewhere)
    int* foam;        // per cell: the particles' foam at its centre, fixed-point (`liquid/foam`)
    uint* stats;      // the statistics' words
    float4* from_position; // `liquid/sort`: last frame's particles, sorted into `position`, ...
    float4* from_velocity;
    float4* from_affine;
    uint* bin_count;  // per cell: its particles (the sort's)
    uint* bin_start;  // per cell: where its particles start within its block of SCAN cells
    uint* block_start;// per block of SCAN cells: where its particles start
    uint2* bin_slot;  // per particle: its cell and its place among the cell's
    // The pressure's multigrid (`LiquidSolver::cycles` > 0): level 0 is the tank's grid, each
    // next one half as fine; from `mg_group` down, one workgroup solves them all.
    uint4 levels[MG_LEVELS]; // xyz: cells along x, y and z; w: where its cells start in mg_*
    uint mg_levels;   // levels, the tank's grid included
    uint mg_group;
    uint smooth;      // red-black sweeps before and after each level's correction
    float smooth_omega; // their over-relaxation
    uint* mg_kind;    // per cell of levels 1 and on: AIR, LIQUID or SOLID
    float* mg_x;      // the pressure's correction, m/s
    float* mg_b;      // what it must undo: the finer level's residual, restricted
};

struct LiquidPush
{
    LiquidFrame* frame;
    uint substep;     // into `steps`
    uint colour;      // pressure: 0 red, 1 black
    uint level;       // pressure: the multigrid's level
    uint pad;
};

[[vk::push_constant]]
LiquidPush liquid_push;

uint3 pcg3d(uint3 v)
{
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> 16u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

uint mix(uint v)
{
    uint state = v * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

int3 cells_of(LiquidFrame* f)
{
    return int3(f[0].cells.xyz);
}

uint node_index(LiquidFrame* f, int3 n)
{
    return uint(n.x) + (f[0].cells.x + 1u) * (uint(n.y) + (f[0].cells.y + 1u) * uint(n.z));
}

uint cell_index(LiquidFrame* f, int3 c)
{
    return uint(c.x) + f[0].cells.x * (uint(c.y) + f[0].cells.y * uint(c.z));
}

int3 unit(uint a)
{
    return int3(a == 0u ? 1 : 0, a == 1u ? 1 : 0, a == 2u ? 1 : 0);
}

// Whether node `n` is a face of component `a`: 0..cells along `a`, inside the cells across.
bool is_face(LiquidFrame* f, uint a, int3 n)
{
    int3 top = cells_of(f) - 1 + unit(a);
    return all(n >= 0) && all(n <= top);
}

bool inside(LiquidFrame* f, int3 c)
{
    return all(c >= 0) && all(c < cells_of(f));
}

// Whether the point `p` lies in the gate's solid part at substep `s`: its slab across the tank,
// above its bottom, outside its hole where the shutter has uncovered it.
bool in_gate(LiquidFrame* f, float3 p, uint s)
{
    float4 g = f[0].gate;
    if (g.y <= g.x || p.x < g.x || p.x > g.y)
        return false;
    float4 step = f[0].steps[s];
    if (p.y < step.x)
        return false;
    float4 h = f[0].hole;
    if (h.z > 0.0 && p.y < step.z && length(p.yz - h.xy) < h.z)
        return false;
    return true;
}

// Whether cell `c` is solid: the glass round the tank, the gate's cells.
bool solid_cell(LiquidFrame* f, int3 c, uint s)
{
    if (!inside(f, c))
        return true;
    return in_gate(f, (float3(c) + 0.5) * f[0].size.w, s);
}

uint kind_at(LiquidFrame* f, int3 c)
{
    return inside(f, c) ? f[0].kind[cell_index(f, c)] : SOLID;
}

// Where component `a`'s faces sit in a cell: on its side along `a`, in its middle across.
float3 face_offset(uint a)
{
    return float3(0.5) - 0.5 * float3(unit(a));
}

// The particles of the water's block, 2 × 2 × 2 a cell, each nudged by up to a quarter of their
// spacing so the grid's regularity does not show; still, with the hydrostatic pressure.
[shader("compute")]
[numthreads(64, 1, 1)]
void seed_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cells.w)
        return;
    uint3 b = f[0].block.xyz;
    uint in_cell = i / 8u;
    uint sub = i % 8u;
    uint3 c = uint3(in_cell % b.x, (in_cell / b.x) % b.y, in_cell / (b.x * b.y));
    float3 o = float3(float(sub & 1u), float((sub >> 1u) & 1u), float((sub >> 2u) & 1u));
    float3 jitter = (float3(pcg3d(uint3(i, 0x51u, 0xa7u)) >> 8u) * (1.0 / 16777216.0) - 0.5) * 0.5;
    float3 x = (float3(c) + (o + 0.5 + jitter) * 0.5) * f[0].size.w;
    f[0].position[i] = float4(x, 0.0);
    f[0].velocity[i] = float4(0.0);
    f[0].affine[3u * i] = float4(0.0);
    f[0].affine[3u * i + 1u] = float4(0.0);
    f[0].affine[3u * i + 2u] = float4(0.0);
}

[shader("compute")]
[numthreads(64, 1, 1)]
void seed_cells_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cell_count)
        return;
    uint3 n = f[0].cells.xyz;
    uint3 c = uint3(i % n.x, (i / n.x) % n.y, i / (n.x * n.y));
    uint3 b = f[0].block.xyz;
    bool water = all(c < b);
    // The still water's pressure: g·dt more each cell down from its surface (none without
    // gravity down).
    f[0].pressure[i] = water ? float(b.y - c.y) * max(-f[0].gravity.y, 0.0) * f[0].dt : 0.0;
    f[0].kind[i] = water ? LIQUID : AIR;
}

void splat(LiquidFrame* f, uint a, float3 x, float u, float3 c)
{
    float h = f[0].size.w;
    float3 g = x / h - face_offset(a);
    int3 base = int3(floor(g));
    float3 t = g - float3(base);
    uint offset = a * f[0].nodes;
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        int3 n = base + o;
        if (!is_face(f, a, n))
            continue;
        float3 w3 = select(o == 1, t, 1.0 - t);
        float w = w3.x * w3.y * w3.z;
        float m = w * (u + dot(c, (float3(n) - g) * h));
        uint i = offset + node_index(f, n);
        InterlockedAdd(f[0].sums[i], (int64_t(int(round(w * FIX))) << 32) + int64_t(int(round(m * FIX))));
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
void p2g_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cells.w)
        return;
    float3 x = f[0].position[i].xyz;
    float3 v = f[0].velocity[i].xyz;
    for (uint a = 0u; a < 3u; a++)
        splat(f, a, x, v[a], f[0].affine[3u * i + a].xyz);
    float h = f[0].size.w;
    int3 home = clamp(int3(floor(x / h)), int3(0), cells_of(f) - 1);
    InterlockedAdd(f[0].count[cell_index(f, home)], 1u);
    // The density at the cell centres round it.
    float3 g = x / h - 0.5;
    int3 base = int3(floor(g));
    float3 t = g - float3(base);
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        int3 c = base + o;
        if (!inside(f, c))
            continue;
        float3 w3 = select(o == 1, t, 1.0 - t);
        InterlockedAdd(f[0].density[cell_index(f, c)], int(round(w3.x * w3.y * w3.z * FIX)));
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
void faces_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].nodes)
        return;
    uint s = liquid_push.substep;
    uint3 side = f[0].cells.xyz + 1u;
    int3 n = int3(int(i % side.x), int((i / side.x) % side.y), int(i / (side.x * side.y)));
    float3 dv = f[0].gravity.xyz * f[0].dt;
    for (uint a = 0u; a < 3u; a++)
    {
        uint at = a * f[0].nodes + i;
        if (!is_face(f, a, n))
        {
            f[0].face[at] = 0.0;
            continue;
        }
        int3 below = n - unit(a);
        bool glass = !inside(f, below) || !inside(f, n);
        float v;
        if (glass)
            v = 0.0;
        else if (solid_cell(f, below, s) || solid_cell(f, n, s))
            v = a == 1u ? f[0].steps[s].y : 0.0; // the gate's, rising
        else
        {
            int64_t sum = f[0].sums[at];
            int momentum = asint(uint(uint64_t(sum) & 0xFFFFFFFFull));
            int w = int((sum - int64_t(momentum)) >> 32);
            v = w > 0 ? float(momentum) / float(w) : 0.0;
            if (w > 0)
                v += dv[a];
        }
        f[0].face[at] = v;
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
void cells_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cell_count)
        return;
    uint s = liquid_push.substep;
    uint3 n = f[0].cells.xyz;
    int3 c = int3(int(i % n.x), int((i / n.x) % n.y), int(i / (n.x * n.y)));
    uint k = solid_cell(f, c, s) ? SOLID : (f[0].count[i] > 0u ? LIQUID : AIR);
    f[0].kind[i] = k;
    if (k != LIQUID)
    {
        f[0].pressure[i] = 0.0;
        f[0].divergence[i] = 0.0;
        f[0].crowding[i] = 0.0;
        return;
    }
    float outflow = 0.0;
    for (uint a = 0u; a < 3u; a++)
    {
        uint offset = a * f[0].nodes;
        outflow += f[0].face[offset + node_index(f, c + unit(a))] - f[0].face[offset + node_index(f, c)];
    }
    // How crowded its particles are: a crowded cell's spread out, and a sparse one's inside the
    // water close up (one at the surface is part full by nature: left alone, or the surface
    // would pull itself in). Without the second, water that splashed apart stays apart: its
    // sparse cells hold their volume like full ones, and it settles high.
    // A cell against the glass or the gate holds less at rest: no particles beyond the solid
    // reach its centre, an eighth of a full cell's weight per solid side.
    float expected = f[0].rest;
    for (uint a = 0u; a < 3u; a++)
        for (int sign = -1; sign <= 1; sign += 2)
            if (solid_cell(f, c + sign * unit(a), s))
                expected *= 0.875;
    float crowding = float(f[0].density[i]) / FIX / expected - 1.0;
    if (crowding < 0.0)
    {
        for (uint a = 0u; a < 3u && crowding < 0.0; a++)
        {
            for (int sign = -1; sign <= 1; sign += 2)
            {
                int3 m = c + sign * unit(a);
                if (!solid_cell(f, m, s) && f[0].count[cell_index(f, m)] == 0u)
                    crowding = 0.0;
            }
        }
    }
    f[0].divergence[i] = outflow;
    f[0].crowding[i] = crowding;
}

// The pressure: in each liquid cell, its open sides' pressure differences undo its outflow
// (`open · p − Σ p_liquid = b`, the air's pressure 0, the solids' sides closed). Level 0 is the
// tank's grid, its `b` the outflow's opposite; a multigrid's coarser levels solve for the
// correction to the finer one's, `b` its residual restricted. Their cells live in the frame's
// buffers, or the coarse ones in a workgroup's memory (`mg_group_main`): `Levels` reads either.
interface Levels
{
    uint3 cells_of(uint l);
    uint kind(uint l, uint i);
    float x(uint l, uint i);
    float b(uint l, uint i);
    void set_x(uint l, uint i, float v);
    // A coarse cell made by restriction: its kind, what it must undo, its correction 0.
    void set_coarse(uint l, uint i, uint k, float b);
}

uint level_index(uint3 n, int3 c)
{
    return uint(c.x) + n.x * (uint(c.y) + n.y * uint(c.z));
}

uint level_kind<S : Levels>(S s, uint l, int3 c)
{
    uint3 n = s.cells_of(l);
    return all(c >= 0) && all(c < int3(n)) ? s.kind(l, level_index(n, c)) : SOLID;
}

// Red-black Gauss–Seidel on cell `j` of colour `colour`'s half of level `l`.
void relax<S : Levels>(S s, uint l, uint j, uint colour, float omega)
{
    uint3 n = s.cells_of(l);
    uint half_x = (n.x + 1u) / 2u;
    if (j >= half_x * n.y * n.z)
        return;
    int y = int((j / half_x) % n.y);
    int z = int(j / (half_x * n.y));
    int x = int(2u * (j % half_x)) + ((y + z + int(colour)) & 1);
    if (x >= int(n.x))
        return;
    int3 c = int3(x, y, z);
    uint i = level_index(n, c);
    if (s.kind(l, i) != LIQUID)
        return;
    float sum = 0.0;
    float open = 0.0;
    for (uint a = 0u; a < 3u; a++)
    {
        for (int sign = -1; sign <= 1; sign += 2)
        {
            int3 m = c + sign * unit(a);
            uint k = level_kind(s, l, m);
            if (k == SOLID)
                continue;
            open += 1.0;
            if (k == LIQUID)
                sum += s.x(l, level_index(n, m));
        }
    }
    if (open == 0.0)
        return;
    float p = s.x(l, i);
    s.set_x(l, i, p + omega * ((sum + s.b(l, i)) / open - p));
}

// What liquid cell `c` of level `l` has left to undo.
float residual<S : Levels>(S s, uint l, int3 c)
{
    uint3 n = s.cells_of(l);
    float sum = 0.0;
    float open = 0.0;
    for (uint a = 0u; a < 3u; a++)
    {
        for (int sign = -1; sign <= 1; sign += 2)
        {
            int3 m = c + sign * unit(a);
            uint k = level_kind(s, l, m);
            if (k == SOLID)
                continue;
            open += 1.0;
            if (k == LIQUID)
                sum += s.x(l, level_index(n, m));
        }
    }
    uint i = level_index(n, c);
    return s.b(l, i) + sum - open * s.x(l, i);
}

// Coarse cell `i` of level `l` from its eight cells of level `l - 1` (McAdams et al. 2010):
// air if any is (the free surface kept where the fine level has it), else liquid if any is,
// else solid. It must undo their residuals' mean four times over: its cells are twice as wide,
// and the equation's differences are a cell's side squared.
void restrict_cell<S : Levels>(S s, uint l, uint i)
{
    uint3 n = s.cells_of(l);
    uint3 fine = s.cells_of(l - 1u);
    int3 c = int3(int(i % n.x), int((i / n.x) % n.y), int(i / (n.x * n.y)));
    bool air = false;
    bool liquid = false;
    float r = 0.0;
    for (uint d = 0u; d < 8u; d++)
    {
        int3 m = 2 * c + int3(int(d & 1u), int((d >> 1u) & 1u), int(d >> 2u));
        if (any(m >= int3(fine)))
            continue;
        uint k = s.kind(l - 1u, level_index(fine, m));
        if (k == AIR)
            air = true;
        else if (k == LIQUID)
        {
            liquid = true;
            r += residual(s, l - 1u, m);
        }
    }
    uint k = air ? AIR : (liquid ? LIQUID : SOLID);
    s.set_coarse(l, i, k, k == LIQUID ? 0.5 * r : 0.0);
}

// Level `l`'s correction at cell `c` of level `l - 1`: trilinear over the eight nearest coarse
// cells' centres, the air's correction 0, the solids' left out (the walls' sides are closed).
float prolong<S : Levels>(S s, uint l, int3 c)
{
    int3 base = (c - 1) >> 1;
    float3 high = select((c & 1) == 0, float3(0.75), float3(0.25));
    float value = 0.0;
    float total = 0.0;
    uint3 n = s.cells_of(l);
    for (uint d = 0u; d < 8u; d++)
    {
        int3 corner = int3(int(d & 1u), int((d >> 1u) & 1u), int(d >> 2u));
        int3 m = base + corner;
        uint k = level_kind(s, l, m);
        if (k == SOLID)
            continue;
        float3 w3 = select(corner == 1, high, 1.0 - high);
        float w = w3.x * w3.y * w3.z;
        total += w;
        if (k == LIQUID)
            value += w * s.x(l, level_index(n, m));
    }
    return total > 0.0 ? value / total : 0.0;
}

// The levels in the frame's buffers: level 0 the tank's grid, the rest in mg_*.
struct FrameLevels : Levels
{
    LiquidFrame* f;
    uint3 cells_of(uint l)
    {
        return f[0].levels[l].xyz;
    }
    uint kind(uint l, uint i)
    {
        return l == 0u ? f[0].kind[i] : f[0].mg_kind[f[0].levels[l].w + i];
    }
    float x(uint l, uint i)
    {
        return l == 0u ? f[0].pressure[i] : f[0].mg_x[f[0].levels[l].w + i];
    }
    float b(uint l, uint i)
    {
        return l == 0u ? -f[0].divergence[i] : f[0].mg_b[f[0].levels[l].w + i];
    }
    void set_x(uint l, uint i, float v)
    {
        if (l == 0u)
            f[0].pressure[i] = v;
        else
            f[0].mg_x[f[0].levels[l].w + i] = v;
    }
    void set_coarse(uint l, uint i, uint k, float b)
    {
        uint at = f[0].levels[l].w + i;
        f[0].mg_kind[at] = k;
        f[0].mg_b[at] = b;
        f[0].mg_x[at] = 0.0;
    }
}

// The plain sweeps, over-relaxed by `omega` (the multigrid's are `relax_main`'s: this one keeps
// level 0's reads unbranched, an eighth faster).
[shader("compute")]
[numthreads(64, 1, 1)]
void pressure_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint3 n = f[0].cells.xyz;
    uint half_x = (n.x + 1u) / 2u;
    uint j = id.x;
    if (j >= half_x * n.y * n.z)
        return;
    int y = int((j / half_x) % n.y);
    int z = int(j / (half_x * n.y));
    int x = int(2u * (j % half_x)) + ((y + z + int(liquid_push.colour)) & 1);
    if (x >= int(n.x))
        return;
    int3 c = int3(x, y, z);
    uint i = cell_index(f, c);
    if (f[0].kind[i] != LIQUID)
        return;
    float sum = 0.0;
    float open = 0.0;
    for (uint a = 0u; a < 3u; a++)
    {
        for (int sign = -1; sign <= 1; sign += 2)
        {
            int3 m = c + sign * unit(a);
            uint k = kind_at(f, m);
            if (k == SOLID)
                continue;
            open += 1.0;
            if (k == LIQUID)
                sum += f[0].pressure[cell_index(f, m)];
        }
    }
    if (open == 0.0)
        return;
    float p = f[0].pressure[i];
    f[0].pressure[i] = p + f[0].omega * ((sum - f[0].divergence[i]) / open - p);
}

// The multigrid's sweeps on a level in the frame's buffers.
[shader("compute")]
[numthreads(64, 1, 1)]
void relax_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    FrameLevels s = { f };
    relax(s, liquid_push.level, id.x, liquid_push.colour, f[0].smooth_omega);
}

// Level `level`'s cells from level `level - 1`'s.
[shader("compute")]
[numthreads(64, 1, 1)]
void restrict_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    FrameLevels s = { f };
    uint3 n = s.cells_of(liquid_push.level);
    if (id.x < n.x * n.y * n.z)
        restrict_cell(s, liquid_push.level, id.x);
}

// Level `level`'s correction added to level `level - 1`'s liquid cells.
[shader("compute")]
[numthreads(64, 1, 1)]
void prolong_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    FrameLevels s = { f };
    uint l = liquid_push.level;
    uint3 n = s.cells_of(l - 1u);
    uint i = id.x;
    if (i >= n.x * n.y * n.z || s.kind(l - 1u, i) != LIQUID)
        return;
    int3 c = int3(int(i % n.x), int((i / n.x) % n.y), int(i / (n.x * n.y)));
    s.set_x(l - 1u, i, s.x(l - 1u, i) + prolong(s, l, c));
}

// The coarse levels, `mg_group` and below, in one workgroup's memory: a dispatch a level costs
// more than the work of a small one. `mg_group`'s kinds and residual come from the frame's
// buffers (`restrict_main`), and its correction goes back to them.
static const uint GROUP_THREADS = 1024u;
static const uint GROUP_FIRST = 4608u; // cells of `mg_group` at most (30 KB in all: AMD allows 32)
static const uint GROUP_REST = 1024u;  // cells of the levels below it at most
static const uint COARSEST_SWEEPS = 16u;
groupshared float group_x[GROUP_FIRST];
groupshared float rest_x[GROUP_REST];
groupshared float rest_b[GROUP_REST];
groupshared uint rest_kind[GROUP_REST];

struct GroupLevels : Levels
{
    LiquidFrame* f;
    uint first;
    // Where level `l` starts in rest_* (`l` > `first`).
    uint rest(uint l)
    {
        return f[0].levels[l].w - f[0].levels[first + 1u].w;
    }
    uint3 cells_of(uint l)
    {
        return f[0].levels[l].xyz;
    }
    uint kind(uint l, uint i)
    {
        return l == first ? f[0].mg_kind[f[0].levels[l].w + i] : rest_kind[rest(l) + i];
    }
    float x(uint l, uint i)
    {
        return l == first ? group_x[i] : rest_x[rest(l) + i];
    }
    float b(uint l, uint i)
    {
        return l == first ? f[0].mg_b[f[0].levels[l].w + i] : rest_b[rest(l) + i];
    }
    void set_x(uint l, uint i, float v)
    {
        if (l == first)
            group_x[i] = v;
        else
            rest_x[rest(l) + i] = v;
    }
    void set_coarse(uint l, uint i, uint k, float b)
    {
        uint at = rest(l) + i;
        rest_kind[at] = k;
        rest_b[at] = b;
        rest_x[at] = 0.0;
    }
}

uint cell_total(uint3 n)
{
    return n.x * n.y * n.z;
}

void group_sweeps(GroupLevels s, uint l, uint sweeps, uint thread)
{
    uint3 n = s.cells_of(l);
    uint half = (n.x + 1u) / 2u * n.y * n.z;
    float omega = s.f[0].smooth_omega;
    for (uint k = 0u; k < sweeps; k++)
    {
        for (uint colour = 0u; colour < 2u; colour++)
        {
            for (uint j = thread; j < half; j += GROUP_THREADS)
                relax(s, l, j, colour, omega);
            GroupMemoryBarrierWithGroupSync();
        }
    }
}

[shader("compute")]
[numthreads(1024, 1, 1)]
void mg_group_main(uint3 thread_id : SV_GroupThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint t = thread_id.x;
    uint first = f[0].mg_group;
    uint last = f[0].mg_levels - 1u;
    GroupLevels s = { f, first };
    uint count = cell_total(s.cells_of(first));
    for (uint i = t; i < count; i += GROUP_THREADS)
        group_x[i] = 0.0;
    GroupMemoryBarrierWithGroupSync();
    for (uint l = first; l < last; l++)
    {
        group_sweeps(s, l, f[0].smooth, t);
        uint coarse = cell_total(s.cells_of(l + 1u));
        for (uint i = t; i < coarse; i += GROUP_THREADS)
            restrict_cell(s, l + 1u, i);
        GroupMemoryBarrierWithGroupSync();
    }
    group_sweeps(s, last, COARSEST_SWEEPS, t);
    for (uint l = last; l > first; l--)
    {
        uint3 n = s.cells_of(l - 1u);
        for (uint i = t; i < cell_total(n); i += GROUP_THREADS)
        {
            if (s.kind(l - 1u, i) != LIQUID)
                continue;
            int3 c = int3(int(i % n.x), int((i / n.x) % n.y), int(i / (n.x * n.y)));
            s.set_x(l - 1u, i, s.x(l - 1u, i) + prolong(s, l, c));
        }
        GroupMemoryBarrierWithGroupSync();
        group_sweeps(s, l - 1u, f[0].smooth, t);
    }
    uint at = f[0].levels[first].w;
    for (uint i = t; i < count; i += GROUP_THREADS)
        f[0].mg_x[at + i] = group_x[i];
}

[shader("compute")]
[numthreads(64, 1, 1)]
void project_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].nodes)
        return;
    uint3 side = f[0].cells.xyz + 1u;
    int3 n = int3(int(i % side.x), int((i / side.x) % side.y), int(i / (side.x * side.y)));
    for (uint a = 0u; a < 3u; a++)
    {
        if (!is_face(f, a, n))
            continue;
        int3 below = n - unit(a);
        uint lo = kind_at(f, below);
        uint hi = kind_at(f, n);
        if (lo == SOLID || hi == SOLID || (lo != LIQUID && hi != LIQUID))
            continue;
        float p_lo = lo == LIQUID ? f[0].pressure[cell_index(f, below)] : 0.0;
        float p_hi = hi == LIQUID ? f[0].pressure[cell_index(f, n)] : 0.0;
        f[0].face[a * f[0].nodes + i] -= p_hi - p_lo;
    }
}

// Component `a`'s velocity at `x` and its gradient (APIC's c for trilinear weights: the sum of
// the faces' velocities times their weights' gradients, Jiang et al. 2015 §5.3), over the faces
// inside the tank.
float gather(LiquidFrame* f, uint a, float3 x, out float3 c)
{
    float h = f[0].size.w;
    float3 g = x / h - face_offset(a);
    int3 base = int3(floor(g));
    float3 t = g - float3(base);
    uint offset = a * f[0].nodes;
    float sum = 0.0;
    float total = 0.0;
    float u[8];
    float3 grad[8];
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        int3 n = base + o;
        u[k] = 0.0;
        grad[k] = float3(0.0);
        if (!is_face(f, a, n))
            continue;
        float3 w3 = select(o == 1, t, 1.0 - t);
        float3 d3 = select(o == 1, float3(1.0), float3(-1.0)) / h;
        float w = w3.x * w3.y * w3.z;
        u[k] = f[0].face[offset + node_index(f, n)];
        grad[k] = float3(d3.x * w3.y * w3.z, w3.x * d3.y * w3.z, w3.x * w3.y * d3.z);
        sum += w * u[k];
        total += w;
    }
    float v = total > 0.0 ? sum / total : 0.0;
    c = float3(0.0);
    for (uint k = 0u; k < 8u; k++)
        c += (u[k] - v) * grad[k];
    return v;
}

// The particles' density at `x` over a full cell's, trilinear between the cell centres (this
// substep's p2g).
float density_at(LiquidFrame* f, float3 x)
{
    float3 g = x / f[0].size.w - 0.5;
    int3 base = int3(floor(g));
    float3 t = g - float3(base);
    int3 top = cells_of(f) - 1;
    float sum = 0.0;
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        float3 w3 = select(o == 1, t, 1.0 - t);
        sum += w3.x * w3.y * w3.z * float(f[0].density[cell_index(f, clamp(base + o, int3(0), top))]);
    }
    return sum / FIX / f[0].rest;
}

// The drift correction's velocity at `x`: down the crowding's gradient, a share `drift` of it
// undone a substep (its gradient is about the crowding over a cell, so a move of `drift` times
// the crowding times a cell), at most half a cell a substep. The glass round the tank takes the
// crowding beside it, so it pushes nothing into the glass.
float3 drift_velocity(LiquidFrame* f, float3 x)
{
    float h = f[0].size.w;
    float3 g = x / h - 0.5;
    int3 base = int3(floor(g));
    float3 t = g - float3(base);
    int3 top = cells_of(f) - 1;
    float3 grad = float3(0.0);
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        float3 w3 = select(o == 1, t, 1.0 - t);
        float3 d3 = select(o == 1, float3(1.0), float3(-1.0)) / h;
        float crowding = f[0].crowding[cell_index(f, clamp(base + o, int3(0), top))];
        grad += crowding * float3(d3.x * w3.y * w3.z, w3.x * d3.y * w3.z, w3.x * w3.y * d3.z);
    }
    float3 v = -f[0].drift * h * h / f[0].dt * grad;
    float cap = 0.5 * h / f[0].dt;
    float l = length(v);
    return l > cap ? v * (cap / l) : v;
}

// Where a particle going from `from` to `to` is stopped by the gate, if it is.
float3 stop_at_gate(LiquidFrame* f, float3 from, float3 to, uint s, inout float3 v)
{
    float4 g = f[0].gate;
    if (g.y <= g.x)
        return to;
    float margin = 0.01 * f[0].size.w;
    float plane;
    if (from.x <= g.x && to.x > g.x)
        plane = g.x;
    else if (from.x >= g.y && to.x < g.y)
        plane = g.y;
    else if (in_gate(f, to, s))
        plane = from.x < 0.5 * (g.x + g.y) ? g.x : g.y;
    else
        return to;
    float share = abs(to.x - from.x) > 1e-9 ? saturate((plane - from.x) / (to.x - from.x)) : 0.0;
    float3 at = lerp(from, to, share);
    at.x = 0.5 * (g.x + g.y);
    if (!in_gate(f, at, s))
        return to;
    bool left = plane == g.x;
    to.x = left ? g.x - margin : g.y + margin;
    v.x = left ? min(v.x, 0.0) : max(v.x, 0.0);
    return to;
}

[shader("compute")]
[numthreads(64, 1, 1)]
void g2p_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cells.w)
        return;
    uint s = liquid_push.substep;
    float4 held = f[0].position[i];
    float3 x = held.xyz;
    float3 before = f[0].velocity[i].xyz;
    float3 v;
    float3 c[3];
    for (uint a = 0u; a < 3u; a++)
        v[a] = gather(f, a, x, c[a]);
    // Its air: in where its skin is fast and where it is stopped hard, out over `foam_life`.
    float dt = f[0].dt;
    float air = saturate((0.8 - density_at(f, x)) / 0.3);
    float stopped = length(v - before - f[0].gravity.xyz * dt) / dt;
    float taken = FOAM_SKIN * air * max(length(v) - FOAM_SPEED, 0.0)
                + FOAM_HARD * max(stopped - FOAM_IMPACT, 0.0);
    float life = f[0].foam_life;
    float foam = life > 0.0 ? saturate(held.w * exp(-dt / life) + taken * dt) : 0.0;
    float3 to = x + (v + drift_velocity(f, x)) * f[0].dt;
    // Inside the glass by a hundredth of a cell, moving no further into it.
    float margin = 0.01 * f[0].size.w;
    float3 top = f[0].size.xyz - margin;
    for (uint a = 0u; a < 3u; a++)
    {
        if (to[a] < margin)
        {
            to[a] = margin;
            v[a] = max(v[a], 0.0);
        }
        else if (to[a] > top[a])
        {
            to[a] = top[a];
            v[a] = min(v[a], 0.0);
        }
    }
    to = stop_at_gate(f, x, to, s, v);
    f[0].position[i] = float4(to, foam);
    f[0].velocity[i] = float4(v, 0.0);
    for (uint a = 0u; a < 3u; a++)
        f[0].affine[3u * i + a] = float4(c[a], 0.0);
}

// The particles' foam to the cell centres round each (trilinear), once a frame for the drawing.
[shader("compute")]
[numthreads(64, 1, 1)]
void foam_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cells.w)
        return;
    float4 p = f[0].position[i];
    if (p.w <= 1e-3)
        return;
    float3 g = p.xyz / f[0].size.w - 0.5;
    int3 base = int3(floor(g));
    float3 t = g - float3(base);
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        int3 c = base + o;
        if (!inside(f, c))
            continue;
        float3 w3 = select(o == 1, t, 1.0 - t);
        InterlockedAdd(f[0].foam[cell_index(f, c)], int(round(p.w * w3.x * w3.y * w3.z * FIX)));
    }
}

// The statistics the log reads: what is left, how high its middle stands, how far the front
// has run along the floor, its energy, a digest of every particle's state.
[shader("compute")]
[numthreads(64, 1, 1)]
void stats_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cells.w)
        return;
    float3 x = f[0].position[i].xyz;
    float3 v = f[0].velocity[i].xyz;
    uint* st = f[0].stats;
    bool lost = any(isnan(x)) || any(x < 0.0) || any(x > f[0].size.xyz);
    if (lost)
    {
        InterlockedAdd(st[STAT_LOST], 1u);
        return;
    }
    InterlockedAdd(st[STAT_PARTICLES], 1u);
    InterlockedAdd(st[STAT_HEIGHT_MM], uint(x.y * 1000.0 + 0.5));
    // The front: the farthest a particle near the floor has gone, 0.1 mm.
    if (x.y < 0.02)
        InterlockedMax(st[STAT_FRONT], uint(x.x * 10000.0));
    InterlockedAdd(st[STAT_ENERGY], uint(dot(v, v) * 100.0 + 0.5));
    InterlockedMax(st[STAT_SPEED], uint(length(v) * 1000.0));
    if (f[0].gate.y > f[0].gate.x && x.x < f[0].gate.x)
        InterlockedAdd(st[STAT_BEHIND], 1u);
    uint3 bits = asuint(x);
    uint3 vbits = asuint(v);
    // Over the particles' states, whatever their order in the buffer (the sort's order within a
    // cell is the atomics': the states are the same, their places not).
    InterlockedAdd(st[STAT_DIGEST], mix(bits.x ^ mix(bits.y ^ mix(bits.z ^ mix(vbits.x)))));
    InterlockedAdd(st[STAT_DIGEST + 1u], mix(vbits.y ^ mix(vbits.z ^ mix(bits.x + 0x9e3779b9u))));
}

// Per column: where its surface stands (the highest cell at least half full, interpolated to
// half), how much water it holds, and how much outflow the last substep's pressure left in its
// liquid cells against what it had to undo (the solve's residual).
[shader("compute")]
[numthreads(64, 1, 1)]
void level_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint3 n = f[0].cells.xyz;
    if (id.x >= n.x * n.z)
        return;
    int x = int(id.x % n.x);
    int z = int(id.x / n.x);
    uint* st = f[0].stats;
    float full = 0.0;
    float top = -1.0;
    uint cells = 0u;
    float left = 0.0;
    float undone = 0.0;
    float worst = 0.0;
    for (int y = int(n.y) - 1; y >= 0; y--)
    {
        uint i = cell_index(f, int3(x, y, z));
        if (f[0].kind[i] == SOLID)
            continue;
        float rho = float(f[0].density[i]) / FIX / f[0].rest;
        full += min(rho, 1.0);
        if (f[0].kind[i] == LIQUID)
        {
            cells++;
            int3 c = int3(x, y, z);
            float outflow = 0.0;
            for (uint a = 0u; a < 3u; a++)
            {
                uint offset = a * f[0].nodes;
                outflow += f[0].face[offset + node_index(f, c + unit(a))] - f[0].face[offset + node_index(f, c)];
            }
            left += abs(outflow);
            undone += abs(f[0].divergence[i]);
            worst = max(worst, abs(outflow));
        }
        if (top < 0.0 && rho >= 0.5)
        {
            float above = y + 1 < int(n.y) ? float(f[0].density[cell_index(f, int3(x, y + 1, z))]) / FIX / f[0].rest : 0.0;
            top = float(y) + 0.5 + saturate((rho - 0.5) / max(rho - above, 1e-6));
        }
    }
    InterlockedAdd(st[STAT_CELLS], cells);
    InterlockedAdd(st[STAT_VOLUME], uint(full * 1000.0 + 0.5));
    // 10 µm/s units; the worst as its bits (a positive float's order is its bits').
    InterlockedAdd(st[STAT_RESIDUAL], uint(left * 1e5 + 0.5));
    InterlockedAdd(st[STAT_DIVERGENCE], uint(undone * 1e5 + 0.5));
    InterlockedMax(st[STAT_RESIDUAL_MAX], asuint(worst));
    if (top >= 0.0)
    {
        InterlockedAdd(st[STAT_LEVEL], uint(top * f[0].size.w * 10000.0 + 0.5));
        InterlockedAdd(st[STAT_COLUMNS], 1u);
    }
}

// The sort (`liquid/sort`, once a frame): the particles by cell, so a wave's threads touch
// neighbouring faces. As the dam break mixes them, the buffer's neighbours stop being the tank's,
// and the p2g's atomics and the g2p's gathers miss the cache (the dam break's p2g took 2.0 ms a
// frame, the hole's, whose particles keep their order, 1.3). A counting sort: each particle
// takes a place in its cell (`bin_main`), the cells' counts are scanned (`scan_main` within
// blocks of SCAN cells, `scan_blocks_main` over the blocks) and each particle copied to its cell's
// start plus its place (`scatter_main`). The places within a cell come from the atomics, in no
// fixed order: the physics does not mind (the grid's sums are integers, the rest per particle).
static const uint SCAN = 1024u;
groupshared uint s_scan[256];

[shader("compute")]
[numthreads(64, 1, 1)]
void bin_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cells.w)
        return;
    float3 x = f[0].from_position[i].xyz;
    int3 home = clamp(int3(floor(x / f[0].size.w)), int3(0), cells_of(f) - 1);
    uint c = cell_index(f, home);
    uint place;
    InterlockedAdd(f[0].bin_count[c], 1u, place);
    f[0].bin_slot[i] = uint2(c, place);
}

// Exclusive scan of SCAN counts a workgroup (256 threads, 4 counts each): within the block into
// `bin_start`, its total into `block_start`.
[shader("compute")]
[numthreads(256, 1, 1)]
void scan_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint n = f[0].cell_count;
    uint first = group.x * SCAN + thread.x * 4u;
    uint v[4];
    uint sum = 0u;
    for (uint k = 0u; k < 4u; k++)
    {
        v[k] = first + k < n ? f[0].bin_count[first + k] : 0u;
        sum += v[k];
    }
    s_scan[thread.x] = sum;
    GroupMemoryBarrierWithGroupSync();
    // Hillis–Steele over the threads' sums.
    for (uint d = 1u; d < 256u; d *= 2u)
    {
        uint add = thread.x >= d ? s_scan[thread.x - d] : 0u;
        GroupMemoryBarrierWithGroupSync();
        s_scan[thread.x] += add;
        GroupMemoryBarrierWithGroupSync();
    }
    uint running = s_scan[thread.x] - sum;
    for (uint k = 0u; k < 4u; k++)
    {
        if (first + k < n)
            f[0].bin_start[first + k] = running;
        running += v[k];
    }
    if (thread.x == 255u)
        f[0].block_start[group.x] = s_scan[255];
}

// Exclusive scan of the blocks' totals in place, one workgroup (at most 256 × 4 blocks).
[shader("compute")]
[numthreads(256, 1, 1)]
void scan_blocks_main(uint3 thread : SV_GroupThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint blocks = (f[0].cell_count + SCAN - 1u) / SCAN;
    uint first = thread.x * 4u;
    uint v[4];
    uint sum = 0u;
    for (uint k = 0u; k < 4u; k++)
    {
        v[k] = first + k < blocks ? f[0].block_start[first + k] : 0u;
        sum += v[k];
    }
    s_scan[thread.x] = sum;
    GroupMemoryBarrierWithGroupSync();
    for (uint d = 1u; d < 256u; d *= 2u)
    {
        uint add = thread.x >= d ? s_scan[thread.x - d] : 0u;
        GroupMemoryBarrierWithGroupSync();
        s_scan[thread.x] += add;
        GroupMemoryBarrierWithGroupSync();
    }
    uint running = s_scan[thread.x] - sum;
    for (uint k = 0u; k < 4u; k++)
    {
        if (first + k < blocks)
            f[0].block_start[first + k] = running;
        running += v[k];
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
void scatter_main(uint3 id : SV_DispatchThreadID)
{
    LiquidFrame* f = liquid_push.frame;
    uint i = id.x;
    if (i >= f[0].cells.w)
        return;
    uint2 slot = f[0].bin_slot[i];
    uint to = f[0].block_start[slot.x / SCAN] + f[0].bin_start[slot.x] + slot.y;
    f[0].position[to] = f[0].from_position[i];
    f[0].velocity[to] = f[0].from_velocity[i];
    for (uint a = 0u; a < 3u; a++)
        f[0].affine[3u * to + a] = f[0].from_affine[3u * i + a];
}
