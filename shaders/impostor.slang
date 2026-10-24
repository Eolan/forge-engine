// Impostors (D-075 step 4, #302): the bake's frames taken into their atlases and the atlases'
// mips. Driven by crates/forge-render/src/impostor.rs.
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
    uint fresnel;    // with `normals`: the Fresnel atlas's first level (a storage image, rg16f)
};

// How far the bake's camera stands from the mesh's middle, in radii (CAMERA_DISTANCE in
// impostor.rs); its near plane is 2 radii nearer.
static const float CAMERA_DISTANCE = 1000.0;

[[vk::push_constant]]
Push push;

// One atlas texel, premultiplied by its coverage (so its mips are plain averages): the average of
// its `samples`² render texels, each weighted by its coverage (an edge keeps the leaf's colour,
// not the background's), times the share covered, which is its alpha. Normals are averaged as
// vectors. With them, into the Fresnel atlas, premultiplied the same way: the leaves' Fresnel
// weight towards the frame's camera, Schlick's (1 - n·v)⁵ of each leaf averaged
// (`impostor_draw.slang` reflects the sky by it), and the depth, the surface's distance from the
// mesh's middle towards the frame's camera in radii.
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
    float fresnel = 0.0;
    float3 towards = frame_direction(float2(push.tile / FRAME_TEXELS));
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
                float3 normal = s.rgb * 2.0 - 1.0;
                float nv = saturate(dot(normal, towards) / max(length(normal), 1e-6));
                fresnel += s.a * pow(1.0 - nv, 5.0);
            }
        }
    }
    // Normals stay their plain average, shorter where they spread: lit by it, a texel gives the
    // average of its leaves' light (a unit normal lit the spread leaves of a crown as one face).
    float3 value = covered > 0.0 ? sum / covered : float3(0.0);
    float alpha = covered / float(n * n);
    atlas[push.tile + id.xy] = float4(value * alpha, alpha);
    if (push.normals != 0u)
    {
        RWTexture2D<float2> target = g_storage_images_rg16f[NonUniformResourceIndex(push.fresnel)];
        target[push.tile + id.xy] = float2(fresnel, depth) / float(n * n);
    }
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

// The same for the Fresnel atlas (rg16f).
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
