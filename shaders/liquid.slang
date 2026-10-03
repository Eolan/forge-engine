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

static const float FIX = 1048576.0; // fixed-point units of the grid's sums (2^20)
static const uint AIR = 0u;
static const uint LIQUID = 1u;
static const uint SOLID = 2u;
static const uint MAX_SUBSTEPS = 8u;
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

// Must match GpuLiquidFrame in liquid.rs.
struct LiquidFrame
{
    float4 size;      // xyz: the tank's inside, metres; w: a cell's side, metres
    uint4 cells;      // xyz: cells along x, y and z; w: particles
    uint4 block;      // xyz: the water's block at the start, cells from the tank's corner; w: unused
    float4 gate;      // x, y: the gate's faces along x, metres (y <= x: no gate); zw: unused
    float4 hole;      // xy: the hole's centre (its y and z), metres; z: its radius (0: none); w: unused
    float4 steps[MAX_SUBSTEPS]; // per substep: x: the gate's bottom, metres; y: its speed up, m/s;
                                // z: 1 while the hole is plugged; w: the substep's time, s
    float4 gravity;   // xyz: m/s² (the lab's 9.81 down by default; `--liquid-gravity`); w: unused
    float dt;         // a substep, seconds
    float rest;       // a full cell's density at rest (its particles' weight)
    float drift;      // the share of a cell's crowding the drift correction undoes a substep
    float omega;      // the pressure's over-relaxation
    uint nodes;       // (cells.x + 1)(cells.y + 1)(cells.z + 1): each component's faces
    uint cell_count;
    uint pad0;
    uint pad1;
    float4* position; // per particle: xyz, metres from the tank's inside corner
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
    uint* stats;      // the statistics' words
};

struct LiquidPush
{
    LiquidFrame* frame;
    uint substep;     // into `steps`
    uint colour;      // pressure: 0 red, 1 black
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
// above its bottom, outside the hole while the hole is open.
bool in_gate(LiquidFrame* f, float3 p, uint s)
{
    float4 g = f[0].gate;
    if (g.y <= g.x || p.x < g.x || p.x > g.y)
        return false;
    float4 step = f[0].steps[s];
    if (p.y < step.x)
        return false;
    float4 h = f[0].hole;
    if (h.z > 0.0 && step.z == 0.0 && length(p.yz - h.xy) < h.z)
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
    float3 x = f[0].position[i].xyz;
    float3 v;
    float3 c[3];
    for (uint a = 0u; a < 3u; a++)
        v[a] = gather(f, a, x, c[a]);
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
    f[0].position[i] = float4(to, 0.0);
    f[0].velocity[i] = float4(v, 0.0);
    for (uint a = 0u; a < 3u; a++)
        f[0].affine[3u * i + a] = float4(c[a], 0.0);
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
    uint3 bits = asuint(x);
    uint3 vbits = asuint(v);
    InterlockedAdd(st[STAT_DIGEST], mix(i ^ mix(bits.x ^ mix(bits.y ^ mix(bits.z)))));
    InterlockedAdd(st[STAT_DIGEST + 1u], mix(i ^ mix(vbits.x ^ mix(vbits.y ^ mix(vbits.z)))));
}

// Per column: where its surface stands (the highest cell at least half full, interpolated to
// half), and how much water it holds.
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
    for (int y = int(n.y) - 1; y >= 0; y--)
    {
        uint i = cell_index(f, int3(x, y, z));
        if (f[0].kind[i] == SOLID)
            continue;
        float rho = float(f[0].density[i]) / FIX / f[0].rest;
        full += min(rho, 1.0);
        if (f[0].kind[i] == LIQUID)
            cells++;
        if (top < 0.0 && rho >= 0.5)
        {
            float above = y + 1 < int(n.y) ? float(f[0].density[cell_index(f, int3(x, y + 1, z))]) / FIX / f[0].rest : 0.0;
            top = float(y) + 0.5 + saturate((rho - 0.5) / max(rho - above, 1e-6));
        }
    }
    InterlockedAdd(st[STAT_CELLS], cells);
    InterlockedAdd(st[STAT_VOLUME], uint(full * 1000.0 + 0.5));
    if (top >= 0.0)
    {
        InterlockedAdd(st[STAT_LEVEL], uint(top * f[0].size.w * 10000.0 + 0.5));
        InterlockedAdd(st[STAT_COLUMNS], 1u);
    }
}
