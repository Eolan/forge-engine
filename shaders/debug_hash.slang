// Debugging aid (issue #71): an order-independent hash of an image's texels, so that two runs
// can be compared frame by frame without waiting for the GPU. Driven by
// crates/forge-render/src/debug_hash.rs.
import bindless;

struct HashPush
{
    uint* out;   // two words per image: the hash sum, then the count of NaN, infinite or (kind 4) differing texels
    uint image;  // sampled-image index
    uint kind;   // 0: float4, 1: float (depth), 2: uint, 4: float4 compared with `other`, 10: a buffer, 12: per tile
    uint width;
    uint height;
    uint index;  // this image's pair in `out`
    uint other;  // kind 4: the image it should equal
    uint* words; // kind 10: the buffer, `width` words, `height` threads a row; kind 12: a word per 8×8 tile, `height` tiles a row (`other`: the image's height)
};

[[vk::push_constant]]
HashPush hash_push;

uint mix(uint v)
{
    // The one-word PCG hash (Jarzynski and Olano, JCGT 2020, after O'Neill's PCG).
    uint state = v * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void hash_main(uint3 id : SV_DispatchThreadID)
{
    uint h = 0;
    uint bad = 0;
    if (hash_push.kind == 12)
    {
        // A hash per 8×8 tile (#277) into `words`, one word a tile, rows of `height` tiles.
        uint h12 = 0;
        if (id.x < hash_push.width && id.y < uint(hash_push.other))
        {
            float4 v = g_sampled_images[NonUniformResourceIndex(hash_push.image)].Load(int3(id.xy, 0));
            uint4 b = asuint(v);
            uint seed = mix(id.y * 65536u + id.x);
            h12 = mix(b.x ^ seed) + mix(b.y ^ (seed + 1u)) + mix(b.z ^ (seed + 2u)) + mix(b.w ^ (seed + 3u));
        }
        uint wave = WaveActiveSum(h12);
        if (WaveIsFirstLane())
        {
            uint ignored;
            InterlockedAdd(hash_push.words[(id.y / 8u) * hash_push.height + id.x / 8u], wave, ignored);
        }
        return;
    }
    if (hash_push.kind == 10)
    {
        // A buffer's words (#277), each hashed with its index.
        uint i = id.y * hash_push.height + id.x;
        if (i < hash_push.width)
            h = mix(hash_push.words[i] ^ mix(i));
        uint wave = WaveActiveSum(h);
        if (WaveIsFirstLane())
        {
            uint ignored;
            InterlockedAdd(hash_push.out[hash_push.index * 2], wave, ignored);
        }
        return;
    }
    if (hash_push.kind == 7 || hash_push.kind == 8)
    {
        // The range of x over the texels that differ from `other`: the largest of each image's
        // (kind 7) or the smallest (kind 8, inverted), as order-preserving bits.
        uint a = 0;
        uint b = 0;
        if (id.x < hash_push.width && id.y < hash_push.height)
        {
            int3 texel = int3(id.xy, 0);
            float4 v = g_sampled_images[NonUniformResourceIndex(hash_push.image)].Load(texel);
            float4 o = g_sampled_images[NonUniformResourceIndex(hash_push.other)].Load(texel);
            if (any(asuint(v) != asuint(o)))
            {
                uint va = asuint(v.x);
                uint ob = asuint(o.x);
                va = (va & 0x80000000u) != 0 ? ~va : va | 0x80000000u;
                ob = (ob & 0x80000000u) != 0 ? ~ob : ob | 0x80000000u;
                a = hash_push.kind == 7 ? va : ~va;
                b = hash_push.kind == 7 ? ob : ~ob;
            }
        }
        uint wave_a = WaveActiveMax(a);
        uint wave_b = WaveActiveMax(b);
        if (WaveIsFirstLane() && (wave_a | wave_b) != 0)
        {
            uint ignored;
            InterlockedMax(hash_push.out[hash_push.index * 2], wave_a, ignored);
            InterlockedMax(hash_push.out[hash_push.index * 2 + 1], wave_b, ignored);
        }
        return;
    }
    if (hash_push.kind == 5 || hash_push.kind == 6)
    {
        // Where the image differs from `other`: the largest packed coordinate, and the largest
        // of its complement, over the differing texels.
        uint last = 0;
        uint first = 0;
        if (id.x < hash_push.width && id.y < hash_push.height)
        {
            int3 texel = int3(id.xy, 0);
            float4 v = g_sampled_images[NonUniformResourceIndex(hash_push.image)].Load(texel);
            float4 o = g_sampled_images[NonUniformResourceIndex(hash_push.other)].Load(texel);
            if (any(asuint(v) != asuint(o)))
            {
                uint packed = hash_push.kind == 5 ? (id.y << 16) | id.x : (id.x << 16) | id.y;
                last = packed;
                first = ~packed;
            }
        }
        uint wave_last = WaveActiveMax(last);
        uint wave_first = WaveActiveMax(first);
        if (WaveIsFirstLane() && wave_first != 0)
        {
            uint ignored;
            InterlockedMax(hash_push.out[hash_push.index * 2], wave_last, ignored);
            InterlockedMax(hash_push.out[hash_push.index * 2 + 1], wave_first, ignored);
        }
        return;
    }
    if (id.x < hash_push.width && id.y < hash_push.height)
    {
        int3 texel = int3(id.xy, 0);
        uint4 bits = uint4(0);
        if (hash_push.kind == 0 || hash_push.kind == 4)
        {
            float4 v = g_sampled_images[NonUniformResourceIndex(hash_push.image)].Load(texel);
            bad = any(isnan(v) || isinf(v)) ? 1u : 0u;
            bits = asuint(v);
            if (hash_push.kind == 4)
            {
                float4 o = g_sampled_images[NonUniformResourceIndex(hash_push.other)].Load(texel);
                bad = any(asuint(v) != asuint(o)) ? 1u : 0u;
            }
        }
        else if (hash_push.kind == 1)
        {
            float v = g_sampled_images_r[NonUniformResourceIndex(hash_push.image)].Load(texel);
            bad = (isnan(v) || isinf(v)) ? 1u : 0u;
            bits.x = asuint(v);
        }
        else
            bits.x = g_sampled_images_u32[NonUniformResourceIndex(hash_push.image)].Load(texel);
        uint seed = mix(id.y * 65536u + id.x);
        h = mix(bits.x ^ seed) + mix(bits.y ^ (seed + 1u)) + mix(bits.z ^ (seed + 2u)) + mix(bits.w ^ (seed + 3u));
    }
    uint sum = WaveActiveSum(h);
    uint bad_sum = WaveActiveSum(bad);
    if (WaveIsFirstLane())
    {
        uint ignored;
        InterlockedAdd(hash_push.out[hash_push.index * 2], sum, ignored);
        if (bad_sum != 0)
            InterlockedAdd(hash_push.out[hash_push.index * 2 + 1], bad_sum, ignored);
    }
}
