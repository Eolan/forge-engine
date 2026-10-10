// A planet tile's normal map made on the GPU from its heights in integers (D-058, #220): the
// samples' heights come from `planet_heights.slang` (the same bits as the CPU's,
// forge_terrain::Planet::fixed_height), their directions are made again here in integers
// (forge_terrain::planet::fixed_direction), and each texel's normal is taken from the four
// samples around its centre and its coast from its centre's height, as
// forge_terrain::planet::tile_normal_map does; then the mips, each texel the mean of the four
// under it (normal_map_levels). Driven by demos/planet/src/normal_maps.rs; its reference is the
// CPU's map (`planet --check-normal-maps`).
import bindless;
import fixed;

// One map: its samples' warp, heights and face, and its images.
struct MapRecord
{
    int64_t* columns;   // (cos, sin) pairs of the warp, Q61, `side` of them
    int64_t* rows;
    int64_t* heights;   // side × side, Q32 metres before the sea flattens them, rows first
    double radius;      // metres
    double coast_range; // metres over which the coast runs from the sea's floor to the land
    uint sea;
    uint side;          // 2 × the map's size + 1
    int axes[9];        // the face's normal, u and v
    uint storage[9];    // the storage image of each mip
};

struct Push
{
    MapRecord* maps;
    uint level;         // the mip this dispatch makes; 0: the map itself
    uint size;          // texels a side of level 0
};

[[vk::push_constant]]
Push push;

[[vk::binding(1, 0)]]
[[vk::image_format("rgba8")]]
RWTexture2D<float4> g_storage_images_rgba8[];

// forge_terrain::planet::fixed_direction: N cos a cos b + U sin a cos b + V cos a sin b,
// normalised, in Q61.
int64_t3 sample_direction(MapRecord* map, uint i, uint j)
{
    int64_t ca = map.columns[2 * i];
    int64_t sa = map.columns[2 * i + 1];
    int64_t cb = map.rows[2 * j];
    int64_t sb = map.rows[2 * j + 1];
    int64_t terms[3] = { mul61(ca, cb), mul61(sa, cb), mul61(ca, sb) };
    int64_t d[3] = { 0, 0, 0 };
    for (uint t = 0; t < 3; t++)
    {
        for (uint c = 0; c < 3; c++)
        {
            d[c] += int64_t(map.axes[t * 3 + c]) * terms[t];
        }
    }
    int64_t r2 = mul61(d[0], d[0]) + mul61(d[1], d[1]) + mul61(d[2], d[2]);
    int64_t y = rsqrt61(r2);
    return int64_t3(mul61(d[0], y), mul61(d[1], y), mul61(d[2], y));
}

// Sample (i, j) on the ground (metres from the planet's centre) and its height before the sea
// flattens it, as the CPU's GridSampler gives them.
double3 ground_point(MapRecord* map, uint i, uint j, out double height)
{
    int64_t3 d = sample_direction(map, i, j);
    int64_t h = map.heights[j * map.side + i];
    int64_t ground = map.sea != 0 ? max(h, int64_t(0)) : h;
    double one = double(ONE);
    double3 direction = double3(double(d.x) / one, double(d.y) / one, double(d.z) / one);
    height = double(h) / double(METRE);
    return direction * (map.radius + double(ground) / double(METRE));
}

// Level 0: a texel's normal from the samples either side of its centre, along u then v, and its
// coast from its centre's height.
[shader("compute")]
[numthreads(8, 8, 1)]
void normal_map(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.size || id.y >= push.size)
    {
        return;
    }
    MapRecord* map = push.maps + id.z;
    uint ci = 2 * id.x + 1;
    uint cj = 2 * id.y + 1;
    double unused;
    double centre_height;
    double3 centre = ground_point(map, ci, cj, centre_height);
    double3 du = ground_point(map, ci + 1, cj, unused) - ground_point(map, ci - 1, cj, unused);
    double3 dv = ground_point(map, ci, cj + 1, unused) - ground_point(map, ci, cj - 1, unused);
    float3 n = cross(float3(du), float3(dv));
    float len = length(n);
    float3 normal = len > 0.0 ? n / len : float3(normalize(centre));
    float coast = float(0.5 + 0.5 * clamp(centre_height / map.coast_range, -1.0, 1.0));
    g_storage_images_rgba8[NonUniformResourceIndex(map.storage[0])][id.xy] =
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
    MapRecord* map = push.maps + id.z;
    RWTexture2D<float4> below = g_storage_images_rgba8[NonUniformResourceIndex(map.storage[push.level - 1])];
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
    g_storage_images_rgba8[NonUniformResourceIndex(map.storage[push.level])][id.xy] =
        float4(normal * 0.5 + 0.5, coast * 0.25);
}
