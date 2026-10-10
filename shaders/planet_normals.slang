// A planet tile's normal map made on the GPU (D-058, #220): the ground's height at each sample
// as forge_terrain::Planet::height makes it (the elevation grids read through Catmull-Rom, two
// levels blended, the regions faded in, the octaves of detail on the same hash), the normal at
// each texel from its four neighbours, the coast in alpha; then its mips, each texel the mean of
// the four under it. Driven by demos/planet/src/normal_maps.rs. The CPU's map
// (forge_terrain::planet::tile_normal_map) is its reference: `planet --check-normal-maps`.
//
// Positions on the planet are doubles where they must be: an f32 steps half a metre at the
// Earth's radius, as far as the samples of a fine tile stand apart. Consumer GPUs run doubles
// 64 times slower than floats, so the rest is taken from the tile's centre in floats, from what
// the CPU worked out there once a map (`GridJob`, `Lattice`): each sample's offset from the
// centre in metres, its latitude and longitude as small angles from the centre's, its place in
// each elevation level and each octave's lattice from the centre's. Coarse tiles and tiles near
// a pole, where those angles are not small, take the latitude and longitude in doubles.
import bindless;

static const double PI = 3.14159265358979311600;
static const double TAU = 6.28318530717958623200;
static const int NO_DATA = -32768;

// One level of an elevation grid in the samples buffer.
struct Level
{
    uint64_t offset; // samples (i16) before it
    uint width;
    uint height;
};

// An elevation grid (forge_terrain::planet::GpuGrid).
struct Grid
{
    double west;
    double east;
    double south;
    double north;
    double blend;           // a region's fade from its edges, radians
    double detail_width;    // the widest octave, metres
    double detail_strength; // metres
    double detail_gain;
    uint detail_count;
    uint whole;             // 1: the whole body, wrapping round the longitude
    uint level_count;
    uint seeds_offset;      // its octaves' seeds in `seeds`
    Level levels[16];
};

// A grid as a map reads it: the level Elevation::height picks for the map's narrowest
// wavelength and the next, and the map's centre's place in each.
struct GridJob
{
    uint level;
    uint blended;      // 1: blended toward the next level by `t`
    float t;
    float blend;       // the region's fade, radians
    int u_cell[2];     // the centre's column and row in each level, an integer and a fraction
    int v_cell[2];
    float u_frac[2];
    float v_frac[2];
    float u_scale[2];  // columns a radian of longitude, rows a radian of latitude (negative:
    float v_scale[2];  //   rows run south)
    float edge[4];     // the centre's distances to the window's west, east, south, north, radians
};

// An octave's lattice at a map's centre: its cell, the centre's place in it, its fade and its
// inverse width.
struct Lattice
{
    int cell_x, cell_y, cell_z;
    float frac_x, frac_y, frac_z;
    float fade;
    float inv_width;
};

// Octaves a grid may have (the jobs' lattice tables).
static const uint MAX_OCTAVES = 16;

// One map to make (forge_terrain::planet::GpuTileMap).
struct MapJob
{
    double normal_x, normal_y, normal_z;
    double u_x, u_y, u_z;
    double v_x, v_y, v_z;
    double min_wavelength;
    double coast_range;
    double* columns;   // 2 size + 1 tangents of the warp
    double* rows;
    Lattice* lattice;  // grid * MAX_OCTAVES + octave
    GridJob* grids;    // per grid
    double centre_x, centre_y, centre_z; // the map's middle sample's direction
    double lat0, lon0;                   // its latitude and longitude
    uint relative;     // 1: latitude and longitude from the centre's, in floats
    uint storage[9];   // the storage image of each mip
};

struct Push
{
    Grid* grids;
    uint* samples;      // i16 pairs
    uint64_t* seeds;
    MapJob* jobs;
    double rough_above;
    double radius;
    uint grid_count;
    uint sea;
    uint level;         // the mip this dispatch makes; 0: the map itself
    uint size;          // texels a side of level 0
};

[[vk::push_constant]]
Push push;

[[vk::binding(1, 0)]]
[[vk::image_format("rgba8")]]
RWTexture2D<float4> g_storage_images_rgba8[];

// ------------------------------------------------------------------------ arithmetic ---

double smooth_d(double t)
{
    t = clamp(t, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

float smooth_f(float t)
{
    t = saturate(t);
    return t * t * (3.0 - 2.0 * t);
}

// atan for doubles, after musl's (FreeBSD's) s_atan.c: the shader languages have no double
// trigonometry. For the coarse and polar tiles only.
double atan_d(double x)
{
    const double atanhi[4] = { 4.63647609000806093515e-01, 7.85398163397448278999e-01,
                               9.82793723247329054082e-01, 1.57079632679489655800e+00 };
    const double atanlo[4] = { 2.26987774529616870924e-17, 3.06161699786838301793e-17,
                               1.39033110312309984516e-17, 6.12323399573676603587e-17 };
    bool negative = x < 0.0;
    double ax = abs(x);
    int id;
    if (ax < 0.4375)
    {
        id = -1;
    }
    else if (ax < 1.1875)
    {
        if (ax < 0.6875)
        {
            id = 0;
            ax = (2.0 * ax - 1.0) / (2.0 + ax);
        }
        else
        {
            id = 1;
            ax = (ax - 1.0) / (ax + 1.0);
        }
    }
    else if (ax < 2.4375)
    {
        id = 2;
        ax = (ax - 1.5) / (1.0 + 1.5 * ax);
    }
    else
    {
        id = 3;
        ax = -1.0 / ax;
    }
    double z = ax * ax;
    double w = z * z;
    double s1 = z * (3.33333333333329318027e-01 + w * (1.42857142725034663711e-01
        + w * (9.09088713343650656196e-02 + w * (6.66107313738753120669e-02
        + w * (4.97687799461593236017e-02 + w * 1.62858201153657823623e-02)))));
    double s2 = w * (-1.99999999998764832476e-01 + w * (-1.11111104054623557880e-01
        + w * (-7.69187620504482999495e-02 + w * (-5.83357013379057348645e-02
        + w * -3.65315727442169155270e-02))));
    double result = id < 0 ? ax - ax * (s1 + s2) : atanhi[id] - ((ax * (s1 + s2) - atanlo[id]) - ax);
    return negative ? -result : result;
}

double atan2_d(double y, double x)
{
    if (x == 0.0)
    {
        return y > 0.0 ? 0.5 * PI : (y < 0.0 ? -0.5 * PI : 0.0);
    }
    double z = atan_d(abs(y / x));
    if (x > 0.0)
    {
        return y < 0.0 ? -z : z;
    }
    return y < 0.0 ? -(PI - z) : PI - z;
}

// ------------------------------------------------------------------------- the noise ---

// forge_core::hash::pcg4d and mix64, to the bit.
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

static const float3 GRADIENTS3[12] = {
    float3(1, 1, 0), float3(-1, 1, 0), float3(1, -1, 0), float3(-1, -1, 0),
    float3(1, 0, 1), float3(-1, 0, 1), float3(1, 0, -1), float3(-1, 0, -1),
    float3(0, 1, 1), float3(0, -1, 1), float3(0, 1, -1), float3(0, -1, -1),
};

float fade(float t)
{
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// forge_procgen::noise::gradient3 at the lattice point `base + offset`: the centre's cell and
// the sample's place from it.
float gradient3(uint64_t seed, int3 base, float3 offset)
{
    float3 whole = floor(offset);
    float3 f = offset - whole;
    int3 i = base + int3(whole);
    float corner[8];
    for (int k = 0; k < 8; k++)
    {
        int3 d = int3(k & 1, (k >> 1) & 1, k >> 2);
        uint64_t h = hash_cell3(seed, i.x + d.x, i.y + d.y, i.z + d.z);
        corner[k] = dot(GRADIENTS3[uint(h % 12)], f - float3(d));
    }
    float3 t = float3(fade(f.x), fade(f.y), fade(f.z));
    float near = lerp(lerp(corner[0], corner[1], t.x), lerp(corner[2], corner[3], t.x), t.y);
    float far = lerp(lerp(corner[4], corner[5], t.x), lerp(corner[6], corner[7], t.x), t.y);
    return lerp(near, far, t.z);
}

// forge_terrain::planet::octaves_by, at `offset` metres from the map's centre.
float octaves(Grid g, uint grid, MapJob job, float3 offset)
{
    float sum = 0.0;
    float norm = 0.0;
    float strength = 1.0;
    float gain = float(g.detail_gain);
    for (uint octave = 0; octave < g.detail_count; octave++)
    {
        norm += strength;
        Lattice l = job.lattice[grid * MAX_OCTAVES + octave];
        if (l.fade > 0.0)
        {
            float3 at = float3(l.frac_x, l.frac_y, l.frac_z) + offset * l.inv_width;
            int3 cell = int3(l.cell_x, l.cell_y, l.cell_z);
            sum += strength * l.fade * gradient3(push.seeds[g.seeds_offset + octave], cell, at);
        }
        strength *= gain;
    }
    return sum / norm;
}

// ----------------------------------------------------------------- the elevation grids ---

int elevation_at(Level l, int x, int y)
{
    uint64_t at = l.offset + uint64_t(y) * l.width + uint64_t(x);
    uint word = push.samples[uint(at >> 1)];
    uint shift = uint(at & 1) * 16u;
    return int(word << (16u - shift)) >> 16;
}

float4 catmull_rom(float t)
{
    float t2 = t * t;
    float t3 = t2 * t;
    return float4(0.5 * (-t3 + 2.0 * t2 - t), 0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
                  0.5 * (-3.0 * t3 + 4.0 * t2 + t), 0.5 * (t3 - t2));
}

// Elevation::sample's taps: the grid at column `ix` + `fx` and row `iy` + `fy`; false where a
// region has no sample.
bool grid_taps(Grid g, Level l, int ix, int iy, float fx, float fy, out double height)
{
    height = 0.0;
    int w = int(l.width);
    int h = int(l.height);
    if (g.whole == 0 && (ix < 1 || iy < 1 || ix + 2 >= w || iy + 2 >= h))
    {
        return false;
    }
    float4 wx = catmull_rom(fx);
    float4 wy = catmull_rom(fy);
    float sum = 0.0;
    for (int j = 0; j < 4; j++)
    {
        float row = 0.0;
        for (int i = 0; i < 4; i++)
        {
            int x = ix + i - 1;
            x = ((x % w) + w) % w;
            int y = clamp(iy + j - 1, 0, h - 1);
            int value = elevation_at(l, x, y);
            if (g.whole == 0 && value == NO_DATA)
            {
                return false;
            }
            row += wx[i] * float(value);
        }
        sum += wy[j] * row;
    }
    height = double(sum);
    return true;
}

// The grid at a sample `dlat`, `dlon` radians from the map's centre, at the levels the job
// picked (Elevation::height).
bool grid_height_rel(Grid g, GridJob gj, float dlat, float dlon, out double height)
{
    height = 0.0;
    double here = 0.0;
    for (uint k = 0; k < 1 + gj.blended; k++)
    {
        float u = gj.u_frac[k] + dlon * gj.u_scale[k];
        float v = gj.v_frac[k] + dlat * gj.v_scale[k];
        float ux = floor(u);
        float vy = floor(v);
        double h;
        if (!grid_taps(g, g.levels[gj.level + k], gj.u_cell[k] + int(ux), gj.v_cell[k] + int(vy), u - ux, v - vy, h))
        {
            return false;
        }
        here = k == 0 ? h : here + (h - here) * double(gj.t);
    }
    height = here;
    return true;
}

// The grid at latitude `lat` and longitude `lon` in doubles, for the coarse and polar tiles.
bool grid_height_abs(Grid g, GridJob gj, double lat, double lon, out double height)
{
    height = 0.0;
    double here = 0.0;
    for (uint k = 0; k < 1 + gj.blended; k++)
    {
        Level l = g.levels[gj.level + k];
        double u;
        double v;
        if (g.whole != 0)
        {
            u = (lon / TAU + 0.5) * double(l.width) - 0.5;
            v = (0.5 - lat / PI) * double(l.height) - 0.5;
        }
        else
        {
            u = (lon - g.west) / (g.east - g.west) * double(l.width) - 0.5;
            v = (g.north - lat) / (g.north - g.south) * double(l.height) - 0.5;
        }
        double x0 = floor(u);
        double y0 = floor(v);
        double h;
        if (!grid_taps(g, l, int(x0), int(y0), float(u - x0), float(v - y0), h))
        {
            return false;
        }
        here = k == 0 ? h : here + (h - here) * double(gj.t);
    }
    height = here;
    return true;
}

double with_detail(Grid g, uint grid, MapJob job, double base, float3 offset)
{
    double rough = 0.25 + 0.75 * smooth_d(base / push.rough_above);
    return base + g.detail_strength * rough * double(octaves(g, grid, job, offset));
}

// Planet::height in `direction`, a sample of `job`'s map: the whole map with its detail, the
// regions over it, faded in from their edges.
double planet_height(MapJob job, double3 direction)
{
    double3 centre = double3(job.centre_x, job.centre_y, job.centre_z);
    float3 offset = float3((direction - centre) * push.radius);
    float dlat = 0.0;
    float dlon = 0.0;
    double lat = 0.0;
    double lon = 0.0;
    if (job.relative != 0)
    {
        // The angles from the centre's: the sine and cosine of each difference in doubles
        // (products of near-equal numbers), their angle in floats.
        double h = sqrt(direction.x * direction.x + direction.z * direction.z);
        double h0 = sqrt(centre.x * centre.x + centre.z * centre.z);
        dlon = atan2(float(direction.x * centre.z - direction.z * centre.x),
                     float(direction.x * centre.x + direction.z * centre.z));
        dlat = atan2(float(direction.y * h0 - h * centre.y), float(h * h0 + direction.y * centre.y));
    }
    else
    {
        lat = atan2_d(direction.y, sqrt(direction.x * direction.x + direction.z * direction.z));
        lon = atan2_d(direction.x, direction.z);
    }
    Grid whole_grid = push.grids[0];
    double whole;
    bool found = job.relative != 0
        ? grid_height_rel(whole_grid, job.grids[0], dlat, dlon, whole)
        : grid_height_abs(whole_grid, job.grids[0], lat, lon, whole);
    if (!found)
    {
        whole = 0.0;
    }
    bool have = false;
    double height = 0.0;
    for (uint r = 1; r < push.grid_count; r++)
    {
        Grid g = push.grids[r];
        GridJob gj = job.grids[r];
        // Elevation::inside: how far into the window, faded over its blend.
        float edge;
        if (job.relative != 0)
        {
            edge = min(min(gj.edge[0] + dlon, gj.edge[1] - dlon), min(gj.edge[2] + dlat, gj.edge[3] - dlat));
        }
        else
        {
            edge = float(min(min(lon - g.west, g.east - lon), min(lat - g.south, g.north - lat)));
        }
        double weight = double(smooth_f(edge / max(gj.blend, 1e-9)));
        if (weight <= 0.0)
        {
            continue;
        }
        double fine;
        found = job.relative != 0
            ? grid_height_rel(g, gj, dlat, dlon, fine)
            : grid_height_abs(g, gj, lat, lon, fine);
        if (!found)
        {
            continue;
        }
        double regional = fine > 0.5 ? with_detail(g, r, job, fine, offset) : min(whole, 0.0) - 2.0;
        if (weight >= 1.0)
        {
            height = regional;
        }
        else
        {
            double under = have ? height : with_detail(whole_grid, 0, job, whole, offset);
            height = under + (regional - under) * weight;
        }
        have = true;
    }
    return have ? height : with_detail(whole_grid, 0, job, whole, offset);
}

// ----------------------------------------------------------------------- the passes ---

// The point of sample (i, j) on the ground and its height before the sea flattens it.
double3 ground_point(MapJob job, uint i, uint j, out double height)
{
    double3 d = double3(job.normal_x, job.normal_y, job.normal_z)
        + double3(job.u_x, job.u_y, job.u_z) * job.columns[i]
        + double3(job.v_x, job.v_y, job.v_z) * job.rows[j];
    d /= sqrt(dot(d, d));
    height = planet_height(job, d);
    double ground = push.sea != 0 ? max(height, 0.0) : height;
    return d * (push.radius + ground);
}

// Level 0: a texel's normal from the samples either side of its centre, along u then v, and its
// coast from its centre's height (tile_normal_map).
[shader("compute")]
[numthreads(8, 8, 1)]
void normal_map(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.size || id.y >= push.size)
    {
        return;
    }
    MapJob job = push.jobs[id.z];
    uint ci = 2 * id.x + 1;
    uint cj = 2 * id.y + 1;
    double unused;
    double centre_height;
    double3 centre = ground_point(job, ci, cj, centre_height);
    double3 du = ground_point(job, ci + 1, cj, unused) - ground_point(job, ci - 1, cj, unused);
    double3 dv = ground_point(job, ci, cj + 1, unused) - ground_point(job, ci, cj - 1, unused);
    float3 n = cross(float3(du), float3(dv));
    float len = length(n);
    float3 normal = len > 0.0 ? n / len : float3(normalize(centre));
    float coast = float(0.5 + 0.5 * clamp(centre_height / job.coast_range, -1.0, 1.0));
    g_storage_images_rgba8[NonUniformResourceIndex(job.storage[0])][id.xy] =
        float4(normal * 0.5 + 0.5, coast);
}

// Level `push.level`: each texel the mean of the four under it, normalised, and their coasts'
// mean (normal_map_levels).
[shader("compute")]
[numthreads(8, 8, 1)]
void normal_mip(uint3 id : SV_DispatchThreadID)
{
    uint side = push.size >> push.level;
    if (id.x >= side || id.y >= side)
    {
        return;
    }
    MapJob job = push.jobs[id.z];
    RWTexture2D<float4> below = g_storage_images_rgba8[NonUniformResourceIndex(job.storage[push.level - 1])];
    float3 sum = 0.0;
    float coast = 0.0;
    for (uint k = 0; k < 4; k++)
    {
        float4 t = below[id.xy * 2 + uint2(k & 1, k >> 1)];
        sum += t.xyz * 2.0 - 1.0;
        coast += t.w;
    }
    float len = length(sum);
    float3 normal = len > 0.0 ? sum / len : float3(0.0, 0.0, 1.0);
    g_storage_images_rgba8[NonUniformResourceIndex(job.storage[push.level])][id.xy] =
        float4(normal * 0.5 + 0.5, coast * 0.25);
}
