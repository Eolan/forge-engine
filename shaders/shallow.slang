// The GPU's shallow-water layer (Phase 3's step 8, D-044 item 5): a finer grid of water columns
// that shadows the authoritative column model (`forge_physics::shallow::Pool`, on the CPU),
// driven by crates/forge-render/src/shallow.rs.
//
// The scheme is the column model's (after Müller-Fischer 2008, "Fast Water Simulation for Games
// Using Height Fields"), each step in two halves of four passes (the flows through
// the faces, each velocity times its upwind depth, worked out where needed):
// - `advect_main`: the faces' velocities carried along by themselves (semi-Lagrangian);
// - `give_main`: the share of its outflow each cell can give without going below zero;
// - `apply_main`: the depths after the scaled flows;
// - `front_main`: a dry cell the water runs into takes up its speed on its far face; then the
//   surface's slope across each face speeds it up, the bed slows it.
// Each pass writes buffers it does not read (or only its own element), with no atomics: the
// same bits every run on one GPU.
//
// Once a frame, after the steps, `shadow_means_main` and `shadow_main` pull each block of
// `ratio` × `ratio` fine cells towards its column's depth and the fine faces towards the
// columns' velocities: the layer adds detail between the columns but never drifts from them.
// `samples_main` writes what the water pass draws (`pool_vert_main` in water.slang).
// Visual only: nothing reads the layer back.

import bindless;

// Water thinner than this, metres, counts as dry (the column model's `DRY`).
static const float DRY = 0.002;
// The most a face's velocity carries a half step, in cells (the column model's).
static const float MOST_CELLS_A_STEP = 0.5;

struct ShallowPush
{
    float* depth_in;     // per fine cell
    float* depth_out;
    float* u_in;         // (nx + 1) × nz faces along x, face x of a row on cell x's −x side
    float* w_in;         // nx × (nz + 1) faces along z
    float* u_out;
    float* w_out;
    float* give;         // per fine cell: the share of its outflow it gives; the means' scratch
    float* columns;      // the column model's depths, u faces, w faces, floats' thickness and bed
    float4* samples;     // what the water pass draws, per fine cell
    uint nx;             // fine cells along x and z
    uint nz;
    uint ratio;          // fine cells along a column's side
    float spacing;       // metres between fine cells
    float dt;            // seconds, this half step
    float gravity;       // m/s², downwards
    float friction;      // velocity lost a second to the bed
    float rate;          // the share of the gap to the columns closed a frame
};

[[vk::push_constant]]
ShallowPush push;

uint cx() { return push.nx / push.ratio; }
uint cz() { return push.nz / push.ratio; }

// The columns' arrays, laid out one after another in `columns`.
float column_depth(uint2 c) { return push.columns[c.y * cx() + c.x]; }
float column_u(uint x, uint z) { return push.columns[cx() * cz() + z * (cx() + 1) + x]; }
float column_w(uint x, uint z) { return push.columns[cx() * cz() + (cx() + 1) * cz() + z * cx() + x]; }
float column_floats(uint2 c) { return push.columns[cx() * cz() + (cx() + 1) * cz() + cx() * (cz() + 1) + c.y * cx() + c.x]; }
float column_bed(uint2 c) { return push.columns[2 * cx() * cz() + (cx() + 1) * cz() + cx() * (cz() + 1) + c.y * cx() + c.x]; }

// A fine cell's bed: its column's (the lab's walls and blocks stand on the columns' edges).
float bed(uint x, uint z) { return column_bed(uint2(x, z) / push.ratio); }

uint cell(uint x, uint z) { return z * push.nx + x; }
uint u_face(uint x, uint z) { return z * (push.nx + 1) + x; }
uint w_face(uint x, uint z) { return z * push.nx + x; }
uint2 column_of(uint x, uint z) { return uint2(x, z) / push.ratio; }

// What floats over fine cell (x, z), bilinearly between the columns' thickness: the water
// pushed aside round a body in a rounded patch, not in its columns' squares.
float floats_at(uint x, uint z)
{
    float2 p = (float2(x, z) + 0.5) / float(push.ratio) - 0.5;
    p = clamp(p, float2(0.0), float2(cx() - 1, cz() - 1));
    uint2 c = min(uint2(p), uint2(cx() - 2, cz() - 2));
    float2 t = p - float2(c);
    float a = lerp(column_floats(c), column_floats(c + uint2(1, 0)), t.x);
    float b = lerp(column_floats(c + uint2(0, 1)), column_floats(c + uint2(1, 1)), t.x);
    return lerp(a, b, t.y);
}

// The surface on fine cell (x, z) from `depth`, with what floats in its column.
float surface(float* depth, uint x, uint z)
{
    uint i = cell(x, z);
    return bed(x, z) + depth[i] + floats_at(x, z);
}

// The column model's `arriving`: the velocity that reaches a face, from upwind.
float arriving(float before, float own, float after)
{
    if (before > 0.0 && before > own)
        return before;
    if (after < 0.0 && after < own)
        return after;
    return own;
}

// `values` (nx × nz of them) sampled bilinearly at (x, z) in their own units, clamped.
float sample_grid(float* values, uint nx, uint nz, float x, float z)
{
    x = clamp(x, 0.0, float(nx - 1));
    z = clamp(z, 0.0, float(nz - 1));
    uint x0 = min(uint(x), nx - 2);
    uint z0 = min(uint(z), nz - 2);
    float tx = x - float(x0);
    float tz = z - float(z0);
    float a00 = values[z0 * nx + x0];
    float a10 = values[z0 * nx + x0 + 1];
    float a01 = values[(z0 + 1) * nx + x0];
    float a11 = values[(z0 + 1) * nx + x0 + 1];
    float a = a00 + (a10 - a00) * tx;
    float b = a01 + (a11 - a01) * tx;
    return a + (b - a) * tz;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void advect_main(uint3 id : SV_DispatchThreadID)
{
    uint x = id.x;
    uint z = id.y;
    uint nx = push.nx;
    uint nz = push.nz;
    float cells = push.dt / push.spacing;
    if (z < nz && x <= nx)
    {
        float v = 0.0;
        if (x >= 1 && x < nx)
        {
            float along = arriving(push.u_in[u_face(x - 1, z)], push.u_in[u_face(x, z)], push.u_in[u_face(x + 1, z)]);
            float across = 0.25 * ((push.w_in[w_face(x - 1, z)] + push.w_in[w_face(x, z)])
                + (push.w_in[w_face(x - 1, z + 1)] + push.w_in[w_face(x, z + 1)]));
            v = sample_grid(push.u_in, nx + 1, nz, float(x) - along * cells, float(z) - across * cells);
        }
        push.u_out[u_face(x, z)] = v;
    }
    if (x < nx && z <= nz)
    {
        float v = 0.0;
        if (z >= 1 && z < nz)
        {
            float along = arriving(push.w_in[w_face(x, z - 1)], push.w_in[w_face(x, z)], push.w_in[w_face(x, z + 1)]);
            float across = 0.25 * ((push.u_in[u_face(x, z - 1)] + push.u_in[u_face(x + 1, z - 1)])
                + (push.u_in[u_face(x, z)] + push.u_in[u_face(x + 1, z)]));
            v = sample_grid(push.w_in, nx, nz + 1, float(x) - across * cells, float(z) - along * cells);
        }
        push.w_out[w_face(x, z)] = v;
    }
}

// The volume through u face (x, z) this half step, from the advected velocities (`u_in`,
// `w_in`) and the depths before it (`depth_in`): the velocity times the upwind depth. Each pass
// that needs it works it out again rather than storing it: a pass and its barrier cost more.
float flux_u(uint x, uint z)
{
    if (x == 0 || x >= push.nx)
        return 0.0;
    float v = push.u_in[u_face(x, z)];
    uint up = v > 0.0 ? cell(x - 1, z) : cell(x, z);
    return v * push.depth_in[up] * push.spacing * push.dt;
}

float flux_w(uint x, uint z)
{
    if (z == 0 || z >= push.nz)
        return 0.0;
    float v = push.w_in[w_face(x, z)];
    uint up = v > 0.0 ? cell(x, z - 1) : cell(x, z);
    return v * push.depth_in[up] * push.spacing * push.dt;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void give_main(uint3 id : SV_DispatchThreadID)
{
    uint x = id.x;
    uint z = id.y;
    if (x >= push.nx || z >= push.nz)
        return;
    float out_volume = (max(flux_u(x + 1, z), 0.0) + max(-flux_u(x, z), 0.0))
        + (max(flux_w(x, z + 1), 0.0) + max(-flux_w(x, z), 0.0));
    float holds = push.depth_in[cell(x, z)] * push.spacing * push.spacing;
    push.give[cell(x, z)] = out_volume > holds && out_volume > 0.0 ? holds / out_volume : 1.0;
}

// The flow through u face (x, z) scaled by its upwind cell's share.
float scaled_u(uint x, uint z)
{
    float f = flux_u(x, z);
    if (x == 0 || x == push.nx)
        return f;
    return f * (f > 0.0 ? push.give[cell(x - 1, z)] : push.give[cell(x, z)]);
}

float scaled_w(uint x, uint z)
{
    float f = flux_w(x, z);
    if (z == 0 || z == push.nz)
        return f;
    return f * (f > 0.0 ? push.give[cell(x, z - 1)] : push.give[cell(x, z)]);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void apply_main(uint3 id : SV_DispatchThreadID)
{
    uint x = id.x;
    uint z = id.y;
    if (x >= push.nx || z >= push.nz)
        return;
    float net = (scaled_u(x, z) - scaled_u(x + 1, z)) + (scaled_w(x, z) - scaled_w(x, z + 1));
    uint i = cell(x, z);
    push.depth_out[i] = max(push.depth_in[i] + net / (push.spacing * push.spacing), 0.0);
}

// Whether fine cell i turned wet this half step.
bool wetted(uint i)
{
    return push.depth_in[i] < DRY && push.depth_out[i] >= DRY;
}

// A dry cell the water runs into takes up its speed: its far face carries the water on as fast
// as it came (the column model's `flow`, its last part). Per face, from the advected velocities
// (`u_in`, `w_in`) into `u_out` and `w_out`: the cell before the face first, then the one after,
// in the column model's order. Then the surface's slope across the face speeds it up and the bed
// slows it (the column model's `accelerate`), from the depths after the flow.
[shader("compute")]
[numthreads(8, 8, 1)]
void front_main(uint3 id : SV_DispatchThreadID)
{
    uint x = id.x;
    uint z = id.y;
    uint nx = push.nx;
    uint nz = push.nz;
    if (z < nz && x <= nx)
    {
        float v = push.u_in[u_face(x, z)];
        // The cell before (x − 1): water came in through its own −x face.
        if (x >= 1 && x < nx && wetted(cell(x - 1, z)) && scaled_u(x - 1, z) > 0.0)
            v = max(v, push.u_in[u_face(x - 1, z)]);
        // The cell after (x): water came in through its own +x face.
        if (x >= 1 && x < nx && wetted(cell(x, z)) && scaled_u(x + 1, z) < 0.0)
            v = min(v, push.u_in[u_face(x + 1, z)]);
        push.u_out[u_face(x, z)] = x >= 1 && x < nx ? accelerated(v, x - 1, z, x, z) : v;
    }
    if (x < nx && z <= nz)
    {
        float v = push.w_in[w_face(x, z)];
        if (z >= 1 && z < nz && wetted(cell(x, z - 1)) && scaled_w(x, z - 1) > 0.0)
            v = max(v, push.w_in[w_face(x, z - 1)]);
        if (z >= 1 && z < nz && wetted(cell(x, z)) && scaled_w(x, z + 1) < 0.0)
            v = min(v, push.w_in[w_face(x, z + 1)]);
        push.w_out[w_face(x, z)] = z >= 1 && z < nz ? accelerated(v, x, z - 1, x, z) : v;
    }
}

// The face between cells a and b (b after a along the axis), velocity v, from the depths after
// the flow (`depth_out`).
float accelerated(float v, uint ax, uint az, uint bx, uint bz)
{
    uint a = cell(ax, az);
    uint b = cell(bx, bz);
    float da = push.depth_out[a];
    float db = push.depth_out[b];
    if (da < DRY && db < DRY)
        return 0.0;
    float ha = surface(push.depth_out, ax, az);
    float hb = surface(push.depth_out, bx, bz);
    float keep = max(1.0 - push.friction * push.dt, 0.0);
    float most = MOST_CELLS_A_STEP * push.spacing / push.dt;
    v = (v - push.dt * push.gravity * (hb - ha) / push.spacing) * keep;
    // Out of a dry cell, or into one whose bed stands over the water: none.
    bool blocked = v > 0.0 ? (da < DRY || bed(bx, bz) > ha) : (db < DRY || bed(ax, az) > hb);
    return blocked ? 0.0 : clamp(v, -most, most);
}

// ---- Shadowing the columns ----------------------------------------------------------------------

// Each column's fine depths' mean into `give` (as scratch), from `depth_in`.
[shader("compute")]
[numthreads(8, 8, 1)]
void shadow_means_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= cx() || id.y >= cz())
        return;
    float sum = 0.0;
    for (uint dz = 0; dz < push.ratio; dz++)
        for (uint dx = 0; dx < push.ratio; dx++)
            sum += push.depth_in[cell(id.x * push.ratio + dx, id.y * push.ratio + dz)];
    push.give[id.y * cx() + id.x] = sum / float(push.ratio * push.ratio);
}

// The columns' velocity along x at fine u face (x, z), bilinearly between theirs.
float columns_u_at(uint x, uint z)
{
    float fx = float(x) / float(push.ratio);
    float fz = (float(z) + 0.5) / float(push.ratio) - 0.5;
    uint n = cx() + 1;
    fx = clamp(fx, 0.0, float(n - 1));
    fz = clamp(fz, 0.0, float(cz() - 1));
    uint x0 = min(uint(fx), n - 2);
    uint z0 = min(uint(fz), cz() - 2);
    float tx = fx - float(x0);
    float tz = fz - float(z0);
    float a = lerp(column_u(x0, z0), column_u(x0 + 1, z0), tx);
    float b = lerp(column_u(x0, z0 + 1), column_u(x0 + 1, z0 + 1), tx);
    return lerp(a, b, tz);
}

float columns_w_at(uint x, uint z)
{
    float fx = (float(x) + 0.5) / float(push.ratio) - 0.5;
    float fz = float(z) / float(push.ratio);
    uint n = cz() + 1;
    fx = clamp(fx, 0.0, float(cx() - 1));
    fz = clamp(fz, 0.0, float(n - 1));
    uint x0 = min(uint(fx), cx() - 2);
    uint z0 = min(uint(fz), n - 2);
    float tx = fx - float(x0);
    float tz = fz - float(z0);
    float a = lerp(column_w(x0, z0), column_w(x0 + 1, z0), tx);
    float b = lerp(column_w(x0, z0 + 1), column_w(x0 + 1, z0 + 1), tx);
    return lerp(a, b, tz);
}

// From `depth_in`, `u_in` and `w_in` into `depth_out`, `u_out` and `w_out`: each fine cell
// moved by `rate` of its column's gap (the same for all its cells, so the column's volume is
// what moves), each face by `rate` of its gap to the columns' velocity. With `rate` 1, the layer
// starts from the columns.
[shader("compute")]
[numthreads(8, 8, 1)]
void shadow_main(uint3 id : SV_DispatchThreadID)
{
    uint x = id.x;
    uint z = id.y;
    uint nx = push.nx;
    uint nz = push.nz;
    if (x < nx && z < nz)
    {
        uint2 c = column_of(x, z);
        uint i = cell(x, z);
        float gap = column_depth(c) - push.give[c.y * cx() + c.x];
        float d = push.rate >= 1.0 ? column_depth(c) : push.depth_in[i] + push.rate * gap;
        push.depth_out[i] = max(d, 0.0);
    }
    if (z < nz && x <= nx)
    {
        float v = push.u_in[u_face(x, z)];
        float target = (x == 0 || x == nx) ? 0.0 : columns_u_at(x, z);
        push.u_out[u_face(x, z)] = push.rate >= 1.0 ? target : v + push.rate * (target - v);
    }
    if (x < nx && z <= nz)
    {
        float v = push.w_in[w_face(x, z)];
        float target = (z == 0 || z == nz) ? 0.0 : columns_w_at(x, z);
        push.w_out[w_face(x, z)] = push.rate >= 1.0 ? target : v + push.rate * (target - v);
    }
}

// What the water pass draws, per fine cell, from `depth_in`, `u_in` and `w_in`: the surface,
// the depth (0: dry), the mean velocity. A dry cell beside water stands no higher than it, so
// the water's edge meets the bed (the column model's `samples`).
[shader("compute")]
[numthreads(8, 8, 1)]
void samples_main(uint3 id : SV_DispatchThreadID)
{
    uint x = id.x;
    uint z = id.y;
    if (x >= push.nx || z >= push.nz)
        return;
    uint i = cell(x, z);
    float d = push.depth_in[i];
    float height = surface(push.depth_in, x, z);
    float2 velocity = float2(0.0);
    if (d >= DRY)
    {
        velocity = 0.5 * float2(push.u_in[u_face(x, z)] + push.u_in[u_face(x + 1, z)],
            push.w_in[w_face(x, z)] + push.w_in[w_face(x, z + 1)]);
    }
    else
    {
        bool found = false;
        float beside = 0.0;
        for (int dz = -1; dz <= 1; dz++)
        {
            for (int dx = -1; dx <= 1; dx++)
            {
                int bx = int(x) + dx;
                int bz = int(z) + dz;
                if (bx < 0 || bz < 0 || bx >= int(push.nx) || bz >= int(push.nz))
                    continue;
                if (push.depth_in[cell(uint(bx), uint(bz))] >= DRY)
                {
                    float s = surface(push.depth_in, uint(bx), uint(bz));
                    beside = found ? max(beside, s) : s;
                    found = true;
                }
            }
        }
        if (found)
            height = min(height, beside);
    }
    push.samples[i] = float4(height, d < DRY ? 0.0 : d, velocity);
}
