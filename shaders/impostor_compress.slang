// The impostors' atlases block-compressed at the bake (D-075 step 4, D-047's BC7 and BC5): one
// thread a 4 × 4 block of one mip level, its 16 bytes into a buffer that the bake copies into
// the atlas and keeps in the derived cache. Driven by crates/forge-render/src/impostor.rs.
//
// - BC7 in mode 6 alone: one subset, RGBA endpoints of 7 bits and a shared low bit each, 4-bit
//   indices. The endpoints first at the block's extremes along its principal axis (power
//   iteration on its covariance), each texel the nearest of the 16 colours between them; then
//   twice the endpoints that fit those indices best, kept while they lower the block's error.
// - BC5, two BC4 blocks: each channel's least and greatest values, each texel the nearest of
//   the 8 between.
import bindless;

// Must match CompressPush in impostor.rs.
struct Push
{
    uint source;    // the level read: a storage image (rgba16f for BC7, rg16f for BC5)
    uint blocks_x;  // the level's blocks a row and a column
    uint blocks_y;
    uint flags;     // BC7: COLOR_SRGB, OPAQUE
    uint4* blocks;  // the level's blocks, row by row
};

// The colour stored in sRGB (a BC7_SRGB atlas; its alpha stays linear).
static const uint COLOR_SRGB = 1u;
// The alpha left out, stored as 1: the line through the block's colours alone.
static const uint OPAQUE = 2u;

[[vk::push_constant]]
Push push;

float3 to_srgb(float3 c)
{
    c = saturate(c);
    return select(c <= 0.0031308, 12.92 * c, 1.055 * pow(c, float3(1.0 / 2.4)) - 0.055);
}

// `count` bits of `value` at bit `at` of `block`, `at` then past them.
void put(inout uint4 block, inout uint at, uint value, uint count)
{
    for (uint i = 0; i < count; i++)
    {
        uint bit = at + i;
        block[bit >> 5] |= ((value >> i) & 1u) << (bit & 31u);
    }
    at += count;
}

// BC7's 4-bit interpolation weights, of 64.
static const uint WEIGHTS4[16] = { 0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64 };

// An endpoint (0 to 1) as mode 6 keeps it: 7 bits a channel and a low bit shared by all four,
// the one of the two that comes nearer; its value back, of 255.
void quantize(float4 e, out uint4 q, out uint p, out float4 back)
{
    float best = 1e30;
    q = uint4(0);
    p = 0;
    back = float4(0.0);
    for (uint low = 0; low < 2; low++)
    {
        uint4 qq = uint4(clamp(round((e * 255.0 - float(low)) * 0.5), 0.0, 127.0));
        float4 v = float4(qq * 2u + low);
        float err = dot(v - e * 255.0, v - e * 255.0);
        if (err < best)
        {
            best = err;
            q = qq;
            p = low;
            back = v;
        }
    }
}

// Each texel (0 to 1) the nearest of the 16 colours between endpoints `r0` and `r1` (of 255):
// their indices, and the block's squared error.
float assign(float4 texel[16], float4 r0, float4 r1, out uint index[16])
{
    float4 palette[16];
    for (uint w = 0; w < 16; w++)
        palette[w] = floor(((64.0 - float(WEIGHTS4[w])) * r0 + float(WEIGHTS4[w]) * r1 + 32.0) / 64.0);
    float total = 0.0;
    for (uint i = 0; i < 16; i++)
    {
        float4 c = texel[i] * 255.0;
        float best = 1e30;
        index[i] = 0;
        for (uint w = 0; w < 16; w++)
        {
            float4 d = palette[w] - c;
            float err = dot(d, d);
            if (err < best)
            {
                best = err;
                index[i] = w;
            }
        }
        total += best;
    }
    return total;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void impostor_bc7_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.blocks_x || id.y >= push.blocks_y)
        return;
    RWTexture2D<float4> source = g_storage_images_rgba16f[NonUniformResourceIndex(push.source)];
    float4 texel[16];
    float4 mean = float4(0.0);
    for (uint i = 0; i < 16; i++)
    {
        float4 c = saturate(source[id.xy * 4 + uint2(i & 3u, i >> 2)]);
        if ((push.flags & OPAQUE) != 0u)
            c.a = 1.0;
        if ((push.flags & COLOR_SRGB) != 0u)
            c.rgb = to_srgb(c.rgb);
        texel[i] = c;
        mean += c;
    }
    mean /= 16.0;
    // The principal axis: power iteration on the covariance, from the bounding box's diagonal.
    float4 lo = texel[0];
    float4 hi = texel[0];
    float4x4 cov = float4x4(0.0);
    for (uint i = 0; i < 16; i++)
    {
        float4 d = texel[i] - mean;
        cov += float4x4(d * d.x, d * d.y, d * d.z, d * d.w);
        lo = min(lo, texel[i]);
        hi = max(hi, texel[i]);
    }
    float4 axis = hi - lo;
    for (uint k = 0; k < 8; k++)
    {
        float4 next = mul(cov, axis);
        float len = length(next);
        if (len < 1e-12)
            break;
        axis = next / len;
    }
    float t0 = 0.0;
    float t1 = 0.0;
    if (dot(axis, axis) > 1e-12)
    {
        axis = normalize(axis);
        t0 = 1e30;
        t1 = -1e30;
        for (uint i = 0; i < 16; i++)
        {
            float t = dot(texel[i] - mean, axis);
            t0 = min(t0, t);
            t1 = max(t1, t);
        }
    }
    uint4 q0;
    uint4 q1;
    uint p0;
    uint p1;
    float4 r0;
    float4 r1;
    quantize(saturate(mean + axis * t0), q0, p0, r0);
    quantize(saturate(mean + axis * t1), q1, p1, r1);
    uint index[16];
    float error = assign(texel, r0, r1, index);
    // Then the endpoints that fit these indices best (least squares, one 2 × 2 system for the
    // four channels), as long as they lower the error.
    for (uint pass = 0; pass < 2; pass++)
    {
        float a = 0.0;
        float b = 0.0;
        float c = 0.0;
        float4 x0 = float4(0.0);
        float4 x1 = float4(0.0);
        for (uint i = 0; i < 16; i++)
        {
            float t = float(WEIGHTS4[index[i]]) / 64.0;
            a += (1.0 - t) * (1.0 - t);
            b += t * (1.0 - t);
            c += t * t;
            x0 += (1.0 - t) * texel[i];
            x1 += t * texel[i];
        }
        float det = a * c - b * b;
        if (abs(det) < 1e-6)
            break;
        uint4 n0;
        uint4 n1;
        uint m0;
        uint m1;
        float4 s0;
        float4 s1;
        quantize(saturate((c * x0 - b * x1) / det), n0, m0, s0);
        quantize(saturate((a * x1 - b * x0) / det), n1, m1, s1);
        uint again[16];
        float refit = assign(texel, s0, s1, again);
        if (refit >= error)
            break;
        error = refit;
        q0 = n0;
        q1 = n1;
        p0 = m0;
        p1 = m1;
        index = again;
    }
    // The first texel's index keeps three bits: its top bit clear, the endpoints swapped if not.
    if (index[0] >= 8u)
    {
        uint4 q = q0;
        q0 = q1;
        q1 = q;
        uint p = p0;
        p0 = p1;
        p1 = p;
        for (uint i = 0; i < 16; i++)
            index[i] = 15u - index[i];
    }
    uint4 block = uint4(0);
    uint at = 0;
    put(block, at, 1u << 6, 7);   // mode 6
    for (uint c = 0; c < 4; c++)
    {
        put(block, at, q0[c], 7);
        put(block, at, q1[c], 7);
    }
    put(block, at, p0, 1);
    put(block, at, p1, 1);
    put(block, at, index[0], 3);
    for (uint i = 1; i < 16; i++)
        put(block, at, index[i], 4);
    push.blocks[id.y * push.blocks_x + id.x] = block;
}

// A BC4 block of 16 values (0 to 255): the greatest and the least as its endpoints, each value
// the nearest of the 8 between (with the first endpoint above the second, the 8-value mode: the
// two, then six between from the first's side; all equal, every index 0).
uint2 bc4_block(float value[16])
{
    float lo = 255.0;
    float hi = 0.0;
    for (uint i = 0; i < 16; i++)
    {
        lo = min(lo, value[i]);
        hi = max(hi, value[i]);
    }
    uint r0 = uint(round(hi));
    uint r1 = uint(round(lo));
    float palette[8];
    palette[0] = float(r0);
    palette[1] = float(r1);
    for (uint k = 2; k < 8; k++)
        palette[k] = (float(8u - k) * float(r0) + float(k - 1u) * float(r1)) / 7.0;
    uint2 block = uint2(r0 | (r1 << 8), 0);
    for (uint i = 0; i < 16; i++)
    {
        uint best = 0;
        float err = 1e30;
        if (r0 > r1)
        {
            for (uint k = 0; k < 8; k++)
            {
                float d = abs(palette[k] - value[i]);
                if (d < err)
                {
                    err = d;
                    best = k;
                }
            }
        }
        uint bit = 16 + 3 * i;
        for (uint b = 0; b < 3; b++)
            block[(bit + b) >> 5] |= ((best >> b) & 1u) << ((bit + b) & 31u);
    }
    return block;
}

// The Fresnel atlas (rg16f): its first channel, 0 to 1 (the premultiplied Fresnel weight), the
// second signed, -1 to 1 (the premultiplied depth), stored as 0.5 + 0.5 × it; the draw reads it
// back as 2g - 1.
[shader("compute")]
[numthreads(8, 8, 1)]
void impostor_bc5_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.blocks_x || id.y >= push.blocks_y)
        return;
    RWTexture2D<float2> source = g_storage_images_rg16f[NonUniformResourceIndex(push.source)];
    float red[16];
    float green[16];
    for (uint i = 0; i < 16; i++)
    {
        float2 v = source[id.xy * 4 + uint2(i & 3u, i >> 2)];
        red[i] = saturate(v.x) * 255.0;
        green[i] = saturate(0.5 + 0.5 * v.y) * 255.0;
    }
    push.blocks[id.y * push.blocks_x + id.x] = uint4(bc4_block(red), bc4_block(green));
}
