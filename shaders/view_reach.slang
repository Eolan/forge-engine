// How far the view reaches (D-088): the geometric mean of the distances a grid of COLUMNS × ROWS of
// the frame's pixels shows, the sky's FAR, written for the CPU to read a few frames later
// (crates/forge-render/src/view_reach.rs). The probes' finest cascade stands at most that far ahead
// (`Probes::set_focus`): round a room near the camera, down a street far from it. One workgroup;
// no wave operation (any subgroup size).
import bindless;

struct Push
{
    float* reach;      // where the reach is written, metres (host-visible)
    uint depth_image;  // sampled: the frame's reversed-Z depth
    uint width;
    uint height;
    float near;        // z = near / depth (an infinite far plane)
};

[[vk::push_constant]]
Push push;

static const uint COLUMNS = 32;
static const uint ROWS = 18;
static const uint THREADS = 64;
static const float NEAREST = 0.1;
static const float FAR = 1000.0;

groupshared float g_sums[THREADS];

[shader("compute")]
[numthreads(64, 1, 1)]
void view_reach_main(uint thread : SV_GroupIndex)
{
    Texture2D<float> depth = g_sampled_images_r[NonUniformResourceIndex(push.depth_image)];
    float sum = 0.0;
    for (uint i = thread; i < COLUMNS * ROWS; i += THREADS)
    {
        uint2 pixel = uint2((float2(float(i % COLUMNS), float(i / COLUMNS)) + 0.5) / float2(COLUMNS, ROWS) * float2(push.width, push.height));
        float d = depth.Load(int3(pixel, 0));
        sum += log2(clamp(d > 0.0 ? push.near / d : FAR, NEAREST, FAR));
    }
    g_sums[thread] = sum;
    GroupMemoryBarrierWithGroupSync();
    for (uint step = THREADS / 2; step > 0; step /= 2)
    {
        if (thread < step)
            g_sums[thread] += g_sums[thread + step];
        GroupMemoryBarrierWithGroupSync();
    }
    if (thread == 0)
        push.reach[0] = exp2(g_sums[0] / float(COLUMNS * ROWS));
}
