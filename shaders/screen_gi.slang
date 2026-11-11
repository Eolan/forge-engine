// The last few metres of the diffuse light, from the screen (D-108, step 4): a visibility bitmask
// at half resolution (after Therrien, Levesque and Gilet, "Screen Space Indirect Lighting with
// Visibility Bitmask", 2023, written here from the paper). Per pixel, a few slices, each a mask of
// 32 sectors of the hemisphere; a sample covers the sectors between its front and its back, a
// guessed thickness away, and the sectors it newly covers bring its light, read from the last
// frame's colour. In the units the shading's ambient light has (irradiance over the sun's).
import bindless;

struct Push
{
    float4x4 previous_from_current; // this frame's clip space to the last's, unjittered
    float2 ndc_to_view_mul;         // as `gtao.slang`
    float2 ndc_to_view_add;
    uint depth_image;   // sampled: the frame's depth
    uint z_image;       // sampled: this pass's view depth, at half resolution
    uint light_image;   // sampled: the last frame's light, at half resolution
    uint input_image;   // sampled: the filter's input, or the temporal pass's
    uint output_image;  // storage
    uint width;         // of what is written
    uint height;
    uint frame;
    float near;
    float radius;       // metres
    float thickness;    // metres
    float pixels;       // half-resolution pixels per metre at a view depth of 1
    float scale;        // the frame's colour to the units kept (`light_main`)
    uint slices;
    uint steps;
    uint reset;         // 1: no history to read, and no last frame's light (a cut, or images new)
};

[[vk::push_constant]]
Push push;

static const float PI = 3.14159265;
static const float SKY_Z = 1e6;
// How much a pixel's history counts on the next frame's.
static const float HISTORY_WEIGHT = 0.9;

float3 view_position(float2 uv, float z)
{
    return float3((uv * push.ndc_to_view_mul + push.ndc_to_view_add) * z, z);
}

float noise_at(uint2 p, uint frame, float2 magic)
{
    float2 q = float2(p) + 5.588238 * float(frame % 64u);
    return frac(52.9829189 * frac(dot(q, magic)));
}

// Half resolution: the nearest of the four pixels' view depth, what the bitmask walks.
[shader("compute")]
[numthreads(8, 8, 1)]
void z_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    Texture2D<float> depth = g_sampled_images_r[NonUniformResourceIndex(push.depth_image)];
    float nearest = 0.0;
    for (uint y = 0u; y < 2u; y++)
    {
        for (uint x = 0u; x < 2u; x++)
            nearest = max(nearest, depth.Load(int3(int2(id.xy * 2u) + int2(x, y), 0)));
    }
    g_storage_images_r32f[NonUniformResourceIndex(push.output_image)][id.xy] = nearest > 0.0 ? push.near / nearest : SKY_Z;
}

// The frame's colour at half resolution, as the next frame's trace reads it: in the units kept, and
// in alpha whether anything was drawn there.
[shader("compute")]
[numthreads(8, 8, 1)]
void light_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    Texture2D<float4> colour = g_sampled_images[NonUniformResourceIndex(push.input_image)];
    Texture2D<float> depth = g_sampled_images_r[NonUniformResourceIndex(push.depth_image)];
    float3 sum = float3(0.0);
    float count = 0.0;
    for (uint y = 0u; y < 2u; y++)
    {
        for (uint x = 0u; x < 2u; x++)
        {
            int2 p = int2(id.xy * 2u) + int2(x, y);
            if (depth.Load(int3(p, 0)) > 0.0)
            {
                sum += colour.Load(int3(p, 0)).rgb;
                count += 1.0;
            }
        }
    }
    float4 out_light = count > 0.0 ? float4(sum / count / max(push.scale, 1e-9), 1.0) : float4(0.0);
    if (any(isnan(out_light)) || any(isinf(out_light)))
        out_light = float4(0.0);
    g_storage_images_rgba16f[NonUniformResourceIndex(push.output_image)][id.xy] = out_light;
}

// What the sample at `uv`, `z` shows in the last frame: rgb, and a whether there is anything.
float4 last_light(float2 uv, float z)
{
    // After a cut the last frame shows another place; on the first frame there is none.
    if (push.reset != 0u)
        return float4(0.0);
    float2 ndc = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    float4 before = mul(push.previous_from_current, float4(ndc, push.near / z, 1.0));
    if (before.w <= 0.0)
        return float4(0.0);
    float2 last_uv = float2(0.5, -0.5) * before.xy / before.w + float2(0.5);
    if (any(last_uv < float2(0.0)) || any(last_uv >= float2(1.0)))
        return float4(0.0);
    return g_sampled_images[NonUniformResourceIndex(push.light_image)].SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], last_uv, 0.0);
}

// The sectors, of 32, between two places, the part of them in [0, 1].
uint sectors(float a, float b)
{
    uint first = uint(floor(saturate(min(a, b)) * 32.0));
    uint end = uint(ceil(saturate(max(a, b)) * 32.0));
    if (first >= 32u || end <= first)
        return 0u;
    uint count = end - first;
    return (count >= 32u ? 0xFFFFFFFFu : (1u << count) - 1u) << first;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void trace_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    RWTexture2D<float4> result = g_storage_images_rgba16f[NonUniformResourceIndex(push.output_image)];
    Texture2D<float> zs = g_sampled_images_r[NonUniformResourceIndex(push.z_image)];
    int2 here = int2(id.xy);
    int2 last = int2(push.width, push.height) - int2(1);
    float z = zs.Load(int3(here, 0));
    if (z >= SKY_Z)
    {
        result[id.xy] = float4(0.0, 0.0, 0.0, z);
        return;
    }
    float2 size = float2(push.width, push.height);
    float2 uv = (float2(id.xy) + float2(0.5)) / size;
    float3 p = view_position(uv, z);
    // The normal from the depth, across the nearer neighbour each way.
    float3 left = view_position(uv - float2(1.0 / size.x, 0.0), zs.Load(int3(clamp(here - int2(1, 0), int2(0), last), 0))) - p;
    float3 right = view_position(uv + float2(1.0 / size.x, 0.0), zs.Load(int3(clamp(here + int2(1, 0), int2(0), last), 0))) - p;
    float3 up = view_position(uv - float2(0.0, 1.0 / size.y), zs.Load(int3(clamp(here - int2(0, 1), int2(0), last), 0))) - p;
    float3 down = view_position(uv + float2(0.0, 1.0 / size.y), zs.Load(int3(clamp(here + int2(0, 1), int2(0), last), 0))) - p;
    float3 across = dot(left, left) < dot(right, right) ? -left : right;
    float3 along = dot(up, up) < dot(down, down) ? -up : down;
    float3 n = normalize(cross(across, along));
    float3 v = normalize(-p);
    if (dot(n, v) < 0.0)
        n = -n;
    float noise = noise_at(id.xy, push.frame, float2(0.06711056, 0.00583715));
    float step_noise = noise_at(id.xy, push.frame, float2(0.00583715, 0.06711056));
    float reach = min(push.radius * push.pixels / z, 48.0);
    float3 sum = float3(0.0);
    for (uint slice = 0u; slice < push.slices; slice++)
    {
        float phi = (float(slice) + noise) / float(push.slices) * PI;
        float2 omega = float2(cos(phi), sin(phi));
        // The slice's plane, through the eye and the pixel: x and y as the view's, z towards the
        // far (the view position's z is positive).
        float3 in_plane = normalize(float3(omega.x, -omega.y, 0.0) - v * dot(float3(omega.x, -omega.y, 0.0), v));
        float3 plane_n = normalize(cross(in_plane, v));
        float3 projected = n - plane_n * dot(n, plane_n);
        float length_p = max(length(projected), 1e-4);
        // The projected normal's angle from the eye's direction, signed towards `in_plane`.
        float angle = sign(dot(projected, in_plane)) * acos(saturate(dot(projected / length_p, v)));
        uint covered = 0u;
        for (int side = -1; side <= 1; side += 2)
        {
            for (uint k = 0u; k < push.steps; k++)
            {
                float s = (float(k) + step_noise) / float(push.steps);
                float2 offset = omega * float(side) * max(s * s * reach, 1.0);
                int2 at = here + int2(round(offset));
                if (any(at < int2(0)) || any(at > last))
                    break;
                float sz = zs.Load(int3(at, 0));
                if (sz >= SKY_Z)
                    continue;
                float2 suv = (float2(at) + float2(0.5)) / size;
                float3 sp = view_position(suv, sz);
                float3 front = sp - p;
                float dist = length(front);
                if (dist < 1e-3 || dist > push.radius)
                    continue;
                float3 back = sp + normalize(sp) * push.thickness - p;
                float front_angle = acos(clamp(dot(front / dist, v), -1.0, 1.0));
                float back_angle = acos(clamp(dot(normalize(back), v), -1.0, 1.0));
                float a = (float(side) * front_angle - angle + PI * 0.5) / PI;
                float b = (float(side) * back_angle - angle + PI * 0.5) / PI;
                uint mask = sectors(a, b);
                uint fresh = mask & ~covered;
                covered |= mask;
                if (fresh == 0u)
                    continue;
                float4 l = last_light(suv, sz);
                if (l.a <= 0.0)
                    continue;
                float t = dist / push.radius;
                float fade = saturate(1.0 - t * t);
                sum += l.rgb * saturate(dot(n, front / dist)) * fade * float(countbits(fresh)) / 32.0;
            }
        }
    }
    float3 light = sum * (PI * 0.5) / float(push.slices);
    if (any(isnan(light)) || any(isinf(light)))
        light = float3(0.0);
    result[id.xy] = float4(light, z);
}

// One spatial pass of a fixed radius: three by three, by how near each is in depth.
[shader("compute")]
[numthreads(8, 8, 1)]
void filter_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    Texture2D<float4> input_image = g_sampled_images[NonUniformResourceIndex(push.input_image)];
    int2 last = int2(push.width, push.height) - int2(1);
    float4 centre = input_image.Load(int3(id.xy, 0));
    float3 sum = float3(0.0);
    float weight = 0.0;
    for (int y = -1; y <= 1; y++)
    {
        for (int x = -1; x <= 1; x++)
        {
            float4 s = input_image.Load(int3(clamp(int2(id.xy) + int2(x, y), int2(0), last), 0));
            if (abs(s.a - centre.a) <= 0.02 * centre.a)
            {
                sum += s.rgb;
                weight += 1.0;
            }
        }
    }
    g_storage_images_rgba16f[NonUniformResourceIndex(push.output_image)][id.xy] = float4(sum / max(weight, 1.0), centre.a);
}

// The one temporal pass: each pixel's light blended over what its place held last frame, where its
// depth says it is the same surface.
[shader("compute")]
[numthreads(8, 8, 1)]
void temporal_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    float4 now = g_sampled_images[NonUniformResourceIndex(push.input_image)].Load(int3(id.xy, 0));
    RWTexture2D<float4> result = g_storage_images_rgba16f[NonUniformResourceIndex(push.output_image)];
    if (push.reset != 0u || now.a >= SKY_Z)
    {
        result[id.xy] = now;
        return;
    }
    float2 uv = (float2(id.xy) + float2(0.5)) / float2(push.width, push.height);
    float2 ndc = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    float4 before = mul(push.previous_from_current, float4(ndc, push.near / now.a, 1.0));
    float2 last_uv = before.w > 0.0 ? float2(0.5, -0.5) * before.xy / before.w + float2(0.5) : float2(-1.0);
    if (any(last_uv < float2(0.0)) || any(last_uv >= float2(1.0)))
    {
        result[id.xy] = now;
        return;
    }
    float4 kept = g_sampled_images[NonUniformResourceIndex(push.light_image)].SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], last_uv, 0.0);
    // Another surface, or a value that is no number (kept, it would stay for good).
    if (!(abs(kept.a - now.a) <= 0.05 * now.a) || any(isnan(kept.rgb)) || any(isinf(kept.rgb)))
    {
        result[id.xy] = now;
        return;
    }
    result[id.xy] = float4(lerp(now.rgb, kept.rgb, HISTORY_WEIGHT), now.a);
}
