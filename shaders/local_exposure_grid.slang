// Local exposure's grid (D-067, #275), driven by crates/forge-render/src/local_exposure.rs: this
// frame's resolved HDR image (TAA's or DLAA's) splatted into a bilateral grid of log luminance
// (Chen, Paris & Durand 2007), blurred, and the cells' mean log luminance blurred by a wide
// Gaussian (Patry 2021); the display pass reads both (`local_exposure.slang`). Built and read
// within the frame: nothing here survives it.
//
// The grid's cells hold (Σ w·l, Σ w) over their pixels, divided by the cell's pixel count, in an
// RG16F atlas: a pixel's weight w is split linearly between its two nearest slices; there are no
// X/Y splat weights, the blur smooths the cells' borders (Patry).
import bindless;
import local_exposure;

struct Push
{
    uint src;     // what the pass reads (sampled index): the HDR image, the grid or the means
    uint dst;     // what it writes (storage index): the grid or the means
    uint means;   // the grid pass: the cells' means (storage index)
    uint width;   // the grid pass: the HDR image's size
    uint height;
    uint pad0;
    uint pad1;
    uint pad2;
};

[[vk::push_constant]]
Push push;

// Fixed point for the shared sums (no float atomics): a cell of up to 128 × 128 pixels (a frame
// up to 8192 × 4608) cannot overflow 32 bits, each pixel adding at most 24 · 4096.
static const float WEIGHT_SCALE = 4096.0;
static const float SUM_SCALE = 4096.0;

groupshared uint g_weight[LE_SLICES];
groupshared uint g_sum[LE_SLICES];
groupshared uint g_mean;

// One 16 × 16 group per cell: its pixels counted into the cell's slices in shared memory, then
// the column written. Each group owns its cell, so there are no global atomics.
[shader("compute")]
[numthreads(16, 16, 1)]
void grid_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID, uint index : SV_GroupIndex)
{
    if (index < LE_SLICES)
    {
        g_weight[index] = 0;
        g_sum[index] = 0;
    }
    if (index == 0)
        g_mean = 0;
    GroupMemoryBarrierWithGroupSync();
    uint2 size = uint2(push.width, push.height);
    uint2 cells = uint2(LE_CELLS_X, LE_CELLS_Y);
    uint2 from = group.xy * size / cells;
    uint2 to = (group.xy + 1) * size / cells;
    uint mean = 0;
    for (uint y = from.y + thread.y; y < to.y; y += 16)
    {
        for (uint x = from.x + thread.x; x < to.x; x += 16)
        {
            float3 color = g_sampled_images[NonUniformResourceIndex(push.src)].Load(int3(int(x), int(y), 0)).rgb;
            float l = local_log_luminance(color);
            float t = l - LE_MIN_LOG2;
            float z = local_slice(l);
            uint z0 = uint(z);
            float f = z - float(z0);
            uint ignored;
            InterlockedAdd(g_weight[z0], uint((1.0 - f) * WEIGHT_SCALE + 0.5), ignored);
            InterlockedAdd(g_sum[z0], uint((1.0 - f) * t * SUM_SCALE + 0.5), ignored);
            if (f > 0.0)
            {
                InterlockedAdd(g_weight[z0 + 1], uint(f * WEIGHT_SCALE + 0.5), ignored);
                InterlockedAdd(g_sum[z0 + 1], uint(f * t * SUM_SCALE + 0.5), ignored);
            }
            mean += uint(t * SUM_SCALE + 0.5);
        }
    }
    uint ignored;
    InterlockedAdd(g_mean, mean, ignored);
    GroupMemoryBarrierWithGroupSync();
    float pixels = float(max((to.x - from.x) * (to.y - from.y), 1u));
    if (index < LE_SLICES)
    {
        float w = float(g_weight[index]) / (WEIGHT_SCALE * pixels);
        float s = float(g_sum[index]) / (SUM_SCALE * pixels);
        // Σ w l = Σ w (l − MIN) + MIN Σ w.
        g_storage_images_rg16f[NonUniformResourceIndex(push.dst)][local_atlas_texel(group.xy, index)] = float2(s + LE_MIN_LOG2 * w, w);
    }
    if (index == 0)
    {
        float m = float(g_mean) / (SUM_SCALE * pixels) + LE_MIN_LOG2;
        g_storage_images_rg16f[NonUniformResourceIndex(push.means)][group.xy] = float2(m, 1.0);
    }
}

// The grid's blur across the cells: a Gaussian of σ = 1 cell, three cells each side, renormalised
// at the image's edges.
static const float GRID_GAUSSIAN[4] = { 1.0, 0.60653066, 0.13533528, 0.011108997 };

float2 grid_load(int2 texel)
{
    return g_sampled_images[NonUniformResourceIndex(push.src)].Load(int3(texel, 0)).xy;
}

// One thread per atlas texel: in X.
[shader("compute")]
[numthreads(8, 8, 1)]
void blur_x_main(uint3 id : SV_DispatchThreadID)
{
    uint2 cells = uint2(LE_CELLS_X, LE_CELLS_Y);
    if (id.x >= LE_CELLS_X * LE_TILES_X || id.y >= LE_CELLS_Y * (LE_SLICES / LE_TILES_X))
        return;
    int2 cell = int2(id.xy % cells);
    int2 tile = int2(id.xy - uint2(cell));
    float2 sum = 0.0;
    float weights = 0.0;
    for (int d = -3; d <= 3; d++)
    {
        int x = cell.x + d;
        if (x < 0 || x >= int(LE_CELLS_X))
            continue;
        float w = GRID_GAUSSIAN[abs(d)];
        sum += w * grid_load(tile + int2(x, cell.y));
        weights += w;
    }
    g_storage_images_rg16f[NonUniformResourceIndex(push.dst)][id.xy] = sum / weights;
}

// In Y, and a [1 2 1] / 4 tent across the slices (Patry: a smaller blur in luminance).
[shader("compute")]
[numthreads(8, 8, 1)]
void blur_yz_main(uint3 id : SV_DispatchThreadID)
{
    uint2 cells = uint2(LE_CELLS_X, LE_CELLS_Y);
    if (id.x >= LE_CELLS_X * LE_TILES_X || id.y >= LE_CELLS_Y * (LE_SLICES / LE_TILES_X))
        return;
    uint2 cell = id.xy % cells;
    uint2 tile = id.xy / cells;
    int slice = int(tile.y * LE_TILES_X + tile.x);
    float2 sum = 0.0;
    float weights = 0.0;
    for (int dz = -1; dz <= 1; dz++)
    {
        int s = slice + dz;
        if (s < 0 || s >= int(LE_SLICES))
            continue;
        float wz = dz == 0 ? 2.0 : 1.0;
        int2 origin = int2(local_atlas_texel(uint2(0, 0), uint(s)));
        for (int d = -3; d <= 3; d++)
        {
            int y = int(cell.y) + d;
            if (y < 0 || y >= int(LE_CELLS_Y))
                continue;
            float w = wz * GRID_GAUSSIAN[abs(d)];
            sum += w * grid_load(origin + int2(int(cell.x), y));
            weights += w;
        }
    }
    g_storage_images_rg16f[NonUniformResourceIndex(push.dst)][id.xy] = sum / weights;
}

// The cells' means blurred by a Gaussian of radius 13 cells (σ = 13 / 3), renormalised at the
// edges, by one group holding the 64 × 36 image in shared memory (2 × 9 KB).
static const int MEANS_RADIUS = 13;
static const uint MEANS = LE_CELLS_X * LE_CELLS_Y;

groupshared float g_means[MEANS];
groupshared float g_across[MEANS];
groupshared float g_gaussian[MEANS_RADIUS + 1];

[shader("compute")]
[numthreads(256, 1, 1)]
void means_main(uint index : SV_GroupIndex)
{
    if (index <= uint(MEANS_RADIUS))
    {
        float sigma = float(MEANS_RADIUS) / 3.0;
        g_gaussian[index] = exp(-0.5 * float(index * index) / (sigma * sigma));
    }
    for (uint i = index; i < MEANS; i += 256)
        g_means[i] = g_sampled_images[NonUniformResourceIndex(push.src)].Load(int3(int(i % LE_CELLS_X), int(i / LE_CELLS_X), 0)).x;
    GroupMemoryBarrierWithGroupSync();
    for (uint i = index; i < MEANS; i += 256)
    {
        int x = int(i % LE_CELLS_X);
        int row = int(i - uint(x));
        float sum = 0.0;
        float weights = 0.0;
        for (int d = -MEANS_RADIUS; d <= MEANS_RADIUS; d++)
        {
            int at = x + d;
            if (at < 0 || at >= int(LE_CELLS_X))
                continue;
            float w = g_gaussian[abs(d)];
            sum += w * g_means[row + at];
            weights += w;
        }
        g_across[i] = sum / weights;
    }
    GroupMemoryBarrierWithGroupSync();
    for (uint i = index; i < MEANS; i += 256)
    {
        int x = int(i % LE_CELLS_X);
        int y = int(i / LE_CELLS_X);
        float sum = 0.0;
        float weights = 0.0;
        for (int d = -MEANS_RADIUS; d <= MEANS_RADIUS; d++)
        {
            int at = y + d;
            if (at < 0 || at >= int(LE_CELLS_Y))
                continue;
            float w = g_gaussian[abs(d)];
            sum += w * g_across[at * int(LE_CELLS_X) + x];
            weights += w;
        }
        g_storage_images_rg16f[NonUniformResourceIndex(push.dst)][uint2(x, y)] = float2(sum / weights, 1.0);
    }
}
