// The temporal frame's motion vectors, driven by crates/forge-render/src/taa.rs: what DLSS and
// FSR 3.1 reproject their history with (D-070; Forge's own TAA resolve and sharpening, after
// Karis 2014, Jimenez 2016 and Playdead 2016, were removed when FSR replaced them).
//
// Every frame is drawn with its projection shifted by a sub-pixel jitter. The motion pass says
// where each pixel was the frame before (static scene: depth and the two cameras suffice; the
// movers add theirs, #79).
//
// The motion is a compute pass (#161): as a fullscreen draw, on the serial frame
// (FORGE_ASYNC=0) behind the clouds' compute work, the same draw from the same inputs came out one
// of two ways from one run to the next. Dispatched, it repeats to the bit.
import bindless;

struct MotionPush
{
    float4x4 previous_from_current; // previous unjittered clip from this frame's unjittered clip
    float jitter_x;                 // this frame's jitter in pixels, x right, y down
    float jitter_y;
    uint depth;
    uint width;
    uint height;
    uint motion;                    // the motion image (storage index)
};

[[vk::push_constant]]
MotionPush motion_push;

[shader("compute")]
[numthreads(8, 8, 1)]
void motion_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= motion_push.width || id.y >= motion_push.height)
        return;
    g_storage_images_rg16f[NonUniformResourceIndex(motion_push.motion)][id.xy] = motion_at(int2(id.xy));
}

float2 motion_at(int2 pixel)
{
    float2 size = float2(float(motion_push.width), float(motion_push.height));
    float depth = g_sampled_images_r[NonUniformResourceIndex(motion_push.depth)].Load(int3(pixel, 0));
    // The jittered projection moved the scene by the jitter: this pixel shows what sits that far
    // the other way in the unjittered view.
    float2 uv = (float2(pixel) + 0.5 - float2(motion_push.jitter_x, motion_push.jitter_y)) / size;
    float4 previous = mul(motion_push.previous_from_current, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
    if (previous.w <= 1e-9)
        return float2(4.0, 4.0);
    float2 ndc = previous.xy / previous.w;
    float2 previous_uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return previous_uv - uv;
}
