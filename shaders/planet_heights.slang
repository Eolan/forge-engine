// A planet tile's heights in integers on the GPU (D-058's step 2, D-016's amendment): every
// sample's height as forge_terrain::Planet::fixed_height makes it, to the bit. A port, line for
// line, of that function and its helpers (fixed_grid_height, fixed_taps, fixed_octaves,
// fixed_gradient3, fixed_direction); the constants (forge_terrain::planet::FixedHeights) and
// the warp's columns and rows come from the CPU. `planet --check-heights` compares the bits.
// Driven by demos/planet/src/normal_maps.rs.
import fixed;

static const int NO_DATA = -32768;
static const uint MAX_OCTAVES = 16;
static const uint MAX_GRIDS = 4;
static const uint INV_WIDTH_BITS = 40;

// The elevation's levels (the normal maps' `Grid` records, their `levels` only).
struct Level
{
    uint64_t offset;
    uint width;
    uint height;
};

struct ElevationGrid
{
    double bounds[4];
    double blend;
    double detail_width;
    double detail_strength;
    double detail_gain;
    uint detail_count;
    uint whole;
    uint level_count;
    uint seeds_offset;
    Level levels[16];
};

// forge_terrain::planet::FixedLevel.
struct FixedLevel
{
    uint level;
    uint pad;
    int64_t u_scale;
    int64_t u_offset;
    int64_t v_scale;
    int64_t v_offset;
};

// forge_terrain::planet::FixedOctave.
struct FixedOctave
{
    int64_t inv_width;
    int64_t offset[3];
    int64_t fade;
    int64_t strength;
    uint64_t seed;
};

// forge_terrain::planet::FixedGrid.
struct FixedGrid
{
    int64_t bounds[4];
    int64_t inv_blend;
    int64_t t;
    int64_t strength;
    int64_t inv_norm;
    uint whole;
    uint blended;
    uint octave_count;
    uint pad;
    FixedLevel levels[2];
    FixedOctave octaves[16];
};

// A tile's heights to make: its grid of `side` samples a side, the warp's (cos, sin) of each
// column and row, its face's axes (±1 along one coordinate each), its constants.
struct HeightsJob
{
    int64_t* columns;  // (cos, sin) pairs, Q61
    int64_t* rows;
    int64_t* out;      // side × side heights, Q32 metres, rows first
    int64_t radius;
    int64_t inv_rough;
    uint grid_count;
    uint sea;
    uint side;
    uint pad;
    int axes[9];       // normal, u, v
    uint pad2;
    FixedGrid grids[4];
};

struct Push
{
    HeightsJob* jobs;
    ElevationGrid* elevation;
    uint* samples;      // i16 pairs
    Cordic* cordic;
};

[[vk::push_constant]]
Push push;

int elevation_at(Level l, int64_t x, int64_t y)
{
    uint64_t at = l.offset + uint64_t(y) * l.width + uint64_t(x);
    uint word = push.samples[uint(at >> 1)];
    uint shift = uint(at & 1) * 16u;
    return int(word << (16u - shift)) >> 16;
}

// fixed_taps: Catmull-Rom around column u and row v (Q32 texels), Q32 metres.
bool fixed_taps(uint grid, uint level, bool whole, int64_t u, int64_t v, out int64_t height)
{
    height = 0;
    Level l = push.elevation[grid].levels[level];
    int64_t w = int64_t(l.width);
    int64_t h = int64_t(l.height);
    int64_t x0 = u >> Q32;
    int64_t y0 = v >> Q32;
    if (!whole && (x0 < 1 || y0 < 1 || x0 + 2 >= w || y0 + 2 >= h))
    {
        return false;
    }
    int64_t mask = METRE - 1;
    int64_t wx[4];
    int64_t wy[4];
    for (uint axis = 0; axis < 2; axis++)
    {
        int64_t t = (axis == 0 ? u : v) & mask;
        int64_t t2 = mul32(t, t);
        int64_t t3 = mul32(t2, t);
        int64_t w0 = (-t3 + 2 * t2 - t) >> 1;
        int64_t w1 = (3 * t3 - 5 * t2 + 2 * METRE) >> 1;
        int64_t w2 = (-3 * t3 + 4 * t2 + t) >> 1;
        int64_t w3 = (t3 - t2) >> 1;
        if (axis == 0)
        {
            wx[0] = w0; wx[1] = w1; wx[2] = w2; wx[3] = w3;
        }
        else
        {
            wy[0] = w0; wy[1] = w1; wy[2] = w2; wy[3] = w3;
        }
    }
    int64_t sum = 0;
    for (int j = 0; j < 4; j++)
    {
        int64_t row = 0;
        for (int i = 0; i < 4; i++)
        {
            int64_t x = ((x0 + i - 1) % w + w) % w;
            int64_t y = clamp(y0 + j - 1, int64_t(0), h - 1);
            int value = elevation_at(l, x, y);
            if (!whole && value == NO_DATA)
            {
                return false;
            }
            row += wx[i] * int64_t(value);
        }
        sum += mul32(row, wy[j]);
    }
    height = sum;
    return true;
}

// fixed_grid_height: the grid's levels at lat, lon (Q61), blended; Q32 metres.
bool fixed_grid_height(HeightsJob* job, uint grid, int64_t lat, int64_t lon, out int64_t height)
{
    height = 0;
    FixedGrid g = job.grids[grid];
    int64_t here = 0;
    for (uint k = 0; k < 1 + g.blended; k++)
    {
        FixedLevel l = g.levels[k];
        int64_t u = mul_shr(lon, l.u_scale, Q61) + l.u_offset;
        int64_t v = mul_shr(lat, l.v_scale, Q61) + l.v_offset;
        int64_t h;
        if (!fixed_taps(grid, l.level, g.whole != 0, u, v, h))
        {
            return false;
        }
        here = k == 0 ? h : here + mul32(h - here, g.t);
    }
    height = here;
    return true;
}

static const int GRADIENTS[36] = {
    1, 1, 0, -1, 1, 0, 1, -1, 0, -1, -1, 0,
    1, 0, 1, -1, 0, 1, 1, 0, -1, -1, 0, -1,
    0, 1, 1, 0, -1, 1, 0, 1, -1, 0, -1, -1,
};

// fixed_gradient3 at lattice point q (Q32).
int64_t fixed_gradient3(uint64_t seed, int64_t q[3])
{
    int64_t mask = METRE - 1;
    int cell[3];
    int64_t f[3];
    for (uint c = 0; c < 3; c++)
    {
        cell[c] = int(q[c] >> Q32);
        f[c] = q[c] & mask;
    }
    int64_t corner[8];
    for (uint k = 0; k < 8; k++)
    {
        int3 d = int3(int(k & 1), int((k >> 1) & 1), int(k >> 2));
        uint64_t h = hash_cell3(seed, cell[0] + d.x, cell[1] + d.y, cell[2] + d.z);
        uint g = uint(h % 12) * 3;
        corner[k] = int64_t(GRADIENTS[g]) * (f[0] - int64_t(d.x) * METRE)
            + int64_t(GRADIENTS[g + 1]) * (f[1] - int64_t(d.y) * METRE)
            + int64_t(GRADIENTS[g + 2]) * (f[2] - int64_t(d.z) * METRE);
    }
    int64_t t0 = fade32(f[0]);
    int64_t t1 = fade32(f[1]);
    int64_t t2 = fade32(f[2]);
    int64_t a = corner[0] + mul32(corner[1] - corner[0], t0);
    int64_t b = corner[2] + mul32(corner[3] - corner[2], t0);
    int64_t near = a + mul32(b - a, t1);
    a = corner[4] + mul32(corner[5] - corner[4], t0);
    b = corner[6] + mul32(corner[7] - corner[6], t0);
    int64_t far = a + mul32(b - a, t1);
    return near + mul32(far - near, t2);
}

// forge_core::hash::pcg4d, mix64 and hash_cell3, to the bit.
uint4 pcg4d(uint4 v)
{
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.w;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v.w += v.y * v.z;
    v ^= v >> 16u;
    v.x += v.y * v.w;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v.w += v.y * v.z;
    return v;
}

uint64_t mix64(uint64_t z)
{
    z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ull;
    z = (z ^ (z >> 27)) * 0x94D049BB133111EBull;
    return z ^ (z >> 31);
}

uint64_t hash_cell3(uint64_t seed, int x, int y, int z)
{
    uint4 r = pcg4d(uint4(uint(x), uint(y), uint(z), uint(seed) ^ uint(seed >> 32)));
    return mix64((uint64_t(r.x) << 32) | (uint64_t(r.y) ^ (uint64_t(r.z) << 7)));
}

// fixed_octaves at p (Q32 metres), Q32.
int64_t fixed_octaves(HeightsJob* job, uint grid, int64_t p[3])
{
    FixedGrid g = job.grids[grid];
    int64_t sum = 0;
    for (uint k = 0; k < g.octave_count; k++)
    {
        FixedOctave o = g.octaves[k];
        if (o.fade <= 0)
        {
            continue;
        }
        int64_t q[3];
        for (uint c = 0; c < 3; c++)
        {
            q[c] = mul_shr(p[c], o.inv_width, INV_WIDTH_BITS) + o.offset[c];
        }
        sum += mul32(mul32(o.strength, o.fade), fixed_gradient3(o.seed, q));
    }
    return mul32(sum, g.inv_norm);
}

int64_t with_detail(HeightsJob* job, uint grid, int64_t base, int64_t p[3])
{
    int64_t rough = METRE / 4 + mul32(3 * METRE / 4, smooth32(mul32(base, job.inv_rough)));
    return base + mul32(mul32(job.grids[grid].strength, rough), fixed_octaves(job, grid, p));
}

// Planet::fixed_height: in direction d (a unit vector, Q61), Q32 metres.
int64_t fixed_height(HeightsJob* job, int64_t d[3])
{
    int64_t2 lon_h = atan2_fixed(push.cordic, d[0], d[2]);
    int64_t2 lat_ = atan2_fixed(push.cordic, mul61(d[1], push.cordic.gain), lon_h.y);
    int64_t lon = lon_h.x;
    int64_t lat = lat_.x;
    int64_t p[3];
    for (uint c = 0; c < 3; c++)
    {
        p[c] = mul_shr(d[c], job.radius, Q61);
    }
    int64_t whole;
    if (!fixed_grid_height(job, 0, lat, lon, whole))
    {
        whole = 0;
    }
    bool have = false;
    int64_t height = 0;
    for (uint r = 1; r < job.grid_count; r++)
    {
        FixedGrid g = job.grids[r];
        int64_t edge = min(min(lon - g.bounds[0], g.bounds[1] - lon), min(lat - g.bounds[2], g.bounds[3] - lat));
        int64_t weight = smooth32(mul_shr(edge, g.inv_blend, Q61));
        if (weight <= 0)
        {
            continue;
        }
        int64_t fine;
        if (!fixed_grid_height(job, r, lat, lon, fine))
        {
            continue;
        }
        int64_t regional = fine > METRE / 2 ? with_detail(job, r, fine, p) : min(whole, int64_t(0)) - 2 * METRE;
        if (weight >= METRE)
        {
            height = regional;
        }
        else
        {
            int64_t under = have ? height : with_detail(job, 0, whole, p);
            height = under + mul32(regional - under, weight);
        }
        have = true;
    }
    return have ? height : with_detail(job, 0, whole, p);
}

// fixed_direction from a column's and a row's (cos, sin).
void fixed_direction(HeightsJob* job, int64_t2 column, int64_t2 row, out int64_t d[3])
{
    int64_t terms[3] = { mul61(column.x, row.x), mul61(column.y, row.x), mul61(column.x, row.y) };
    for (uint c = 0; c < 3; c++)
    {
        d[c] = 0;
    }
    for (uint t = 0; t < 3; t++)
    {
        for (uint c = 0; c < 3; c++)
        {
            d[c] += int64_t(job.axes[t * 3 + c]) * terms[t];
        }
    }
    int64_t r2 = mul61(d[0], d[0]) + mul61(d[1], d[1]) + mul61(d[2], d[2]);
    int64_t y = rsqrt61(r2);
    for (uint c = 0; c < 3; c++)
    {
        d[c] = mul61(d[c], y);
    }
}

[shader("compute")]
[numthreads(8, 8, 1)]
void tile_heights(uint3 id : SV_DispatchThreadID)
{
    HeightsJob* job = push.jobs + id.z;
    if (id.x >= job.side || id.y >= job.side)
    {
        return;
    }
    int64_t2 column = int64_t2(job.columns[2 * id.x], job.columns[2 * id.x + 1]);
    int64_t2 row = int64_t2(job.rows[2 * id.y], job.rows[2 * id.y + 1]);
    int64_t d[3];
    fixed_direction(job, column, row, d);
    job.out[id.y * job.side + id.x] = fixed_height(job, d);
}
