// Impostors (D-075 step 4, #302): the bake's frames taken into their atlases, the atlases' mips
// and the occlusion. Driven by crates/forge-render/src/impostor.rs.
import bindless;
import impostor_layout;

// Texels a side of a frame (FRAME_TEXELS in impostor.rs).
static const uint FRAME_TEXELS = 64;

// Must match DownsamplePush in impostor.rs.
struct Push
{
    uint source;     // the frame's render (downsample) or the finer mip (mips): a storage image
    uint atlas;      // the atlas's level written: a storage image (rgba16f)
    uint2 tile;      // downsample: the frame's corner in the atlas, texels; mips: the level's side
    uint samples;    // the render's texels a side per atlas texel
    uint normals;    // 1: the render holds normals (× 0.5 + 0.5)
    uint depth;      // with `normals`: the render's depth (sampled, reversed-Z infinite)
    uint pad;
};

// How far the bake's camera stands from the mesh's middle, in radii (CAMERA_DISTANCE in
// impostor.rs); its near plane is 2 radii nearer.
static const float CAMERA_DISTANCE = 1000.0;

[[vk::push_constant]]
Push push;

// One atlas texel, premultiplied by its coverage (so its mips are plain averages): the average of
// its `samples`² render texels, each weighted by its coverage (an edge keeps the leaf's colour,
// not the background's), times the share covered, which is its alpha. Normals are averaged as
// vectors; their alpha is the depth instead, the surface's distance from the
// mesh's middle towards the frame's camera in radii, averaged and premultiplied the same way (the
// base colour's alpha is the coverage).
[shader("compute")]
[numthreads(8, 8, 1)]
void impostor_downsample_main(uint3 id : SV_DispatchThreadID)
{
    RWTexture2D<float4> source = g_storage_images_rgba16f[NonUniformResourceIndex(push.source)];
    RWTexture2D<float4> atlas = g_storage_images_rgba16f[NonUniformResourceIndex(push.atlas)];
    uint n = push.samples;
    float3 sum = float3(0.0);
    float covered = 0.0;
    float depth = 0.0;
    for (uint y = 0; y < n; y++)
    {
        for (uint x = 0; x < n; x++)
        {
            uint2 at = id.xy * n + uint2(x, y);
            float4 s = source[at];
            sum += s.rgb * s.a;
            covered += s.a;
            if (push.normals != 0u && s.a > 0.0)
            {
                // Reversed-Z infinite: the view distance is near / z.
                float z = g_sampled_images_r[NonUniformResourceIndex(push.depth)].Load(int3(at, 0));
                depth += s.a * (CAMERA_DISTANCE - (CAMERA_DISTANCE - 2.0) / max(z, 1e-6));
            }
        }
    }
    // Normals stay their plain average, shorter where they spread: lit by it, a texel gives the
    // average of its leaves' light (a unit normal lit the spread leaves of a crown as one face).
    float3 value = covered > 0.0 ? sum / covered : float3(0.0);
    float alpha = covered / float(n * n);
    float last = push.normals != 0u ? depth / float(n * n) : alpha;
    atlas[push.tile + id.xy] = float4(value * alpha, last);
}

// One texel of the next mip: the average of its four in the finer one. The frames' sides are
// powers of two, so no average straddles two frames.
[shader("compute")]
[numthreads(8, 8, 1)]
void impostor_mip_main(uint3 id : SV_DispatchThreadID)
{
    if (any(id.xy >= push.tile))
        return;
    RWTexture2D<float4> finer = g_storage_images_rgba16f[NonUniformResourceIndex(push.source)];
    RWTexture2D<float4> level = g_storage_images_rgba16f[NonUniformResourceIndex(push.atlas)];
    uint2 p = id.xy * 2;
    level[id.xy] = 0.25 * (finer[p] + finer[p + uint2(1, 0)] + finer[p + uint2(0, 1)] + finer[p + uint2(1, 1)]);
}

// The same for the occlusion atlas (rg16f).
[shader("compute")]
[numthreads(8, 8, 1)]
void impostor_mip_rg_main(uint3 id : SV_DispatchThreadID)
{
    if (any(id.xy >= push.tile))
        return;
    RWTexture2D<float2> finer = g_storage_images_rg16f[NonUniformResourceIndex(push.source)];
    RWTexture2D<float2> level = g_storage_images_rg16f[NonUniformResourceIndex(push.atlas)];
    uint2 p = id.xy * 2;
    level[id.xy] = 0.25 * (finer[p] + finer[p + uint2(1, 0)] + finer[p + uint2(0, 1)] + finer[p + uint2(1, 1)]);
}

// Must match OcclusionPush in impostor.rs.
struct OcclusionPush
{
    uint albedo;     // the finished atlases, sampled with their mips
    uint normal;
    uint occlusion;  // the occlusion atlas's first level: a storage image (rg16f)
    uint pad;
};

[[vk::push_constant]]
OcclusionPush occlusion_push;

// How far behind the surface a frame saw a point must lie to be in the mesh's own shade, in radii,
// and the mip the frames are read at as shadow maps (`SELF_SHADOW_BIAS` in impostor_draw.slang).
static const float OCCLUSION_BIAS = 0.06;
static const float OCCLUSION_LEVEL = 1.0;

// The share of the sky one texel's point sees past the mesh itself, premultiplied by its coverage:
// every frame read as a shadow map (`frame_occlusion`) from the point the texel's own frame saw,
// weighted by the cosine with its leaves' average normal (a little towards even, where they
// spread). The resolve dims the sky's light and its reflection by the screen's occlusion; far off
// a crown's own shade has to come from here.
[shader("compute")]
[numthreads(8, 8, 1)]
void impostor_occlusion_main(uint3 id : SV_DispatchThreadID)
{
    uint2 texel = id.xy;
    float4 a = g_sampled_images[NonUniformResourceIndex(occlusion_push.albedo)].Load(int3(texel, 0));
    RWTexture2D<float2> target = g_storage_images_rg16f[NonUniformResourceIndex(occlusion_push.occlusion)];
    if (a.a <= 0.0)
    {
        target[texel] = float2(0.0);
        return;
    }
    float4 n4 = g_sampled_images[NonUniformResourceIndex(occlusion_push.normal)].Load(int3(texel, 0));
    float3 normal = n4.rgb / a.a * 2.0 - 1.0;
    // The point the texel's frame saw, in radii: its place in the picture, then its depth.
    float2 frame = float2(texel / FRAME_TEXELS);
    float2 uv = (float2(texel % FRAME_TEXELS) + 0.5) / float(FRAME_TEXELS);
    float3 d = frame_direction(frame);
    float3 right;
    float3 up;
    frame_axes(d, right, up);
    float3 p = right * (uv.x * 2.0 - 1.0) + up * (1.0 - uv.y * 2.0) + d * (n4.a / a.a);
    float seen = 0.0;
    float total = 0.0;
    for (uint k = 0; k < FRAMES * FRAMES; k++)
    {
        float2 other = float2(k % FRAMES, k / FRAMES);
        float w = max(dot(normal, frame_direction(other)), 0.0) + 0.1;
        seen += w * (1.0 - frame_occlusion(occlusion_push.albedo, occlusion_push.normal, other, p, OCCLUSION_BIAS, OCCLUSION_LEVEL));
        total += w;
    }
    target[texel] = float2(seen / total * a.a, 0.0);
}
