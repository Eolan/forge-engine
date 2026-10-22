// Impostors (D-075 step 4, #302): the bake's frames taken into their atlases. Driven by
// crates/forge-render/src/impostor.rs.
import bindless;

// Must match DownsamplePush in impostor.rs.
struct Push
{
    uint source;     // the frame's render: a storage image (rgba16f), alpha its coverage
    uint atlas;      // the atlas: a storage image (rgba16f)
    uint2 tile;      // the frame's corner in the atlas, texels
    uint samples;    // the render's texels a side per atlas texel
    uint normals;    // 1: the render holds normals (× 0.5 + 0.5)
    uint2 pad;
};

[[vk::push_constant]]
Push push;

// One atlas texel: the average of its `samples`² render texels, each weighted by its coverage
// (an edge keeps the leaf's colour, not the background's), the share covered its alpha. Normals
// are averaged as vectors and made unit again.
[shader("compute")]
[numthreads(8, 8, 1)]
void impostor_downsample_main(uint3 id : SV_DispatchThreadID)
{
    RWTexture2D<float4> source = g_storage_images_rgba16f[NonUniformResourceIndex(push.source)];
    RWTexture2D<float4> atlas = g_storage_images_rgba16f[NonUniformResourceIndex(push.atlas)];
    uint n = push.samples;
    float3 sum = float3(0.0);
    float covered = 0.0;
    for (uint y = 0; y < n; y++)
    {
        for (uint x = 0; x < n; x++)
        {
            float4 s = source[id.xy * n + uint2(x, y)];
            sum += s.rgb * s.a;
            covered += s.a;
        }
    }
    float3 value = covered > 0.0 ? sum / covered : float3(0.0);
    if (push.normals != 0u && covered > 0.0)
    {
        float3 n3 = value * 2.0 - 1.0;
        value = dot(n3, n3) > 1e-8 ? normalize(n3) * 0.5 + 0.5 : float3(0.5, 1.0, 0.5);
    }
    atlas[push.tile + id.xy] = float4(value, covered / float(n * n));
}
