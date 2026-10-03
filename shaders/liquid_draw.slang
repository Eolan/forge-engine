// The lab's liquid drawn (#156, D-044): the solver's density ray-marched through the glass tank.
// Driven by crates/forge-render/src/liquid.rs; the solver is liquid.slang.
//
// - `liquid/smooth`: the particles' density at the cell centres, over a full cell's, blurred
//   twice by a 3 × 3 × 3 binomial (a 5-tap binomial in all: a step stays where it was, the
//   particles' noise does not), and the
//   bricks' least and most of it (4 cells a side, with the cell round them that trilinear
//   sampling reaches), so the march jumps over air and over the water's inside;
// - `liquid/copy`: the scene as it stands, sampled where a bent ray lands;
// - `liquid/march`: per pixel, the ray through the tank's glass (a pane's Fresnel reflection and
//   its tint), into the water where the density crosses one half (the normal from its
//   gradient: Fresnel's reflection of the sky and the sun, the rest refracted by Snell's law),
//   absorbed and scattered by the water along its path (Beer–Lambert, pure water's
//   coefficients by default: it barely tints a metre), out through the surface (or reflected
//   whole beyond the critical angle) or through the glass, and on to what the scene shows where
//   it lands. It writes the reactive mask TAA reads where the water moves.

import bindless;
import sh;
import skyview;

static const float PI = 3.14159265358979;
static const uint BRICK = 4u;
static const uint MAX_STEPS = 384u;
static const float SURFACE = 0.5;
// Fresnel's reflectance head-on: water, glass.
static const float WATER_F0 = 0.02;
static const float GLASS_F0 = 0.04;
// The most a moving surface asks of TAA's current frame, and the speed that asks it, m/s.
static const float REACTIVE = 0.5;
static const float REACTIVE_SPEED = 3.0;
// How far behind a surface of the depth a bent ray may be and still have met it, metres.
static const float THICKNESS = 0.25;
// Foam: how much light a full cell's worth of foamy particles scatters and blocks a metre (white
// water is near opaque over a few centimetres), and the least a brick's foam must be to march it.
static const float FOAM_SIGMA = 40.0;
static const float FOAM_EPS = 0.01;
// The speed view's top of scale, m/s.
static const float SPEED_TOP = 3.0;

// Must match GpuLiquidView in liquid.rs.
struct LiquidView
{
    float4x4 inv_view_proj; // clip → camera-relative world (the drawing camera, jitter included)
    float4x4 view_proj;
    float4 tank;            // xyz: the tank's inside corner, camera-relative; w: a cell, metres
    float4 size;            // xyz: the inside, metres; w: the glass's thickness
    uint4 cells;            // xyz: cells along x, y, z; w: unused
    uint4 bricks;           // xyz: bricks along x, y, z; w: unused
    float4 sun;             // xyz: towards the sun; w: the sun's highlight's roughness²
    float4 sun_radiance;    // rgb: the sun's pre-exposed illuminance; w: the sky's luminance scale
    float4 absorption;      // rgb: the liquid's absorption, 1/m; w: its index of refraction
    float4 scattering;      // rgb: its scattering, 1/m; w: unused
    float4 glass;           // rgb: what a pane lets through; w: 1 where the glass shows, 0 where it does not
    float4 background;      // rgb: the plain background's pre-exposed radiance; w: 1 for one, 0 for the sky
    float rest;             // a full cell's density
    uint nodes;             // the faces' grid, per component
    uint color;             // the frame's HDR colour (storage)
    uint scene;             // its copy (sampled)
    uint depth;             // the frame's depth (sampled)
    uint reactive;          // the reactive mask (storage, r32f)
    uint width;
    uint height;
    uint mode;              // 0: the water as it looks; 1: its surface coloured by the flow's speed
    uint pad;
    int* density;           // the solver's, per cell, fixed-point
    float* blur;            // the first of the two smoothing passes'
    float* render;          // smoothed, over a full cell's
    float4* brick_range;    // per brick: least and most of `render`, most of `foam_render`
    float* face;            // the solver's faces' velocities, for the reactive mask
    float4* sky_light;      // the sky's irradiance (SH), then its up and forward (sky.rs)
    int* foam;              // the solver's foam per cell, fixed-point (`liquid/foam`)
    float* foam_render;     // smoothed, over a full cell's worth of foamy particles
};

struct ViewPush
{
    LiquidView* view;
};

[[vk::push_constant]]
ViewPush view_push;

static const float FIX = 1048576.0;

uint cell_index(LiquidView* v, int3 c)
{
    return uint(c.x) + v[0].cells.x * (uint(c.y) + v[0].cells.y * uint(c.z));
}

// A cell's 3 × 3 × 3 binomial blur of `from` (fixed-point, `scale` to a full cell's), the glass
// round the tank taking the water's own beside it (so the water meets the glass full rather than
// fading into it).
float blur3(LiquidView* v, int* from, float scale, int3 c)
{
    int3 n = int3(v[0].cells.xyz);
    float sum = 0.0;
    for (int z = -1; z <= 1; z++)
        for (int y = -1; y <= 1; y++)
            for (int x = -1; x <= 1; x++)
            {
                int3 m = clamp(c + int3(x, y, z), int3(0), n - 1);
                float w = float((2 - abs(x)) * (2 - abs(y)) * (2 - abs(z)));
                sum += w * float(from[cell_index(v, m)]);
            }
    return sum / 64.0 * scale;
}

int3 cell_of(LiquidView* v, uint i)
{
    uint3 n = v[0].cells.xyz;
    return int3(int(i % n.x), int((i / n.x) % n.y), int(i / (n.x * n.y)));
}

[shader("compute")]
[numthreads(64, 1, 1)]
void smooth_main(uint3 id : SV_DispatchThreadID)
{
    LiquidView* v = view_push.view;
    uint3 n = v[0].cells.xyz;
    if (id.x >= n.x * n.y * n.z)
        return;
    v[0].blur[id.x] = blur3(v, v[0].density, 1.0 / FIX / v[0].rest, cell_of(v, id.x));
    v[0].foam_render[id.x] = blur3(v, v[0].foam, 1.0 / FIX / v[0].rest, cell_of(v, id.x));
}

[shader("compute")]
[numthreads(64, 1, 1)]
void smooth2_main(uint3 id : SV_DispatchThreadID)
{
    LiquidView* v = view_push.view;
    uint3 n = v[0].cells.xyz;
    if (id.x >= n.x * n.y * n.z)
        return;
    int3 c = cell_of(v, id.x);
    float sum = 0.0;
    for (int z = -1; z <= 1; z++)
        for (int y = -1; y <= 1; y++)
            for (int x = -1; x <= 1; x++)
            {
                int3 m = clamp(c + int3(x, y, z), int3(0), int3(n) - 1);
                sum += float((2 - abs(x)) * (2 - abs(y)) * (2 - abs(z))) * v[0].blur[cell_index(v, m)];
            }
    v[0].render[id.x] = sum / 64.0;
}

[shader("compute")]
[numthreads(64, 1, 1)]
void bricks_main(uint3 id : SV_DispatchThreadID)
{
    LiquidView* v = view_push.view;
    uint3 b = v[0].bricks.xyz;
    uint i = id.x;
    if (i >= b.x * b.y * b.z)
        return;
    int3 k = int3(int(i % b.x), int((i / b.x) % b.y), int(i / (b.x * b.y)));
    int3 n = int3(v[0].cells.xyz);
    int3 lo = max(k * int(BRICK) - 1, int3(0));
    int3 hi = min(k * int(BRICK) + int(BRICK), n - 1);
    float least = 1e9;
    float most = -1e9;
    float foam = 0.0;
    for (int z = lo.z; z <= hi.z; z++)
        for (int y = lo.y; y <= hi.y; y++)
            for (int x = lo.x; x <= hi.x; x++)
            {
                uint c = cell_index(v, int3(x, y, z));
                float r = v[0].render[c];
                least = min(least, r);
                most = max(most, r);
                foam = max(foam, v[0].foam_render[c]);
            }
    v[0].brick_range[i] = float4(least, most, foam, 0.0);
}

struct CopyPush
{
    uint color;
    uint copy;
    uint width;
    uint height;
    uint depth;   // the frame's depth (sampled)
    uint plain;   // 1: what the depth leaves empty takes `background`
    uint pad0;
    uint pad1;
    float4 background;
};

[[vk::push_constant]]
CopyPush copy_push;

[shader("compute")]
[numthreads(8, 8, 1)]
void copy_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= copy_push.width || id.y >= copy_push.height)
        return;
    float4 c = g_sampled_images[copy_push.color].Load(int3(id.xy, 0));
    if (copy_push.plain != 0u && g_sampled_images_r[copy_push.depth].Load(int3(id.xy, 0)) <= 0.0)
        c = float4(copy_push.background.rgb, 1.0);
    g_storage_images_rgba16f[copy_push.copy][id.xy] = c;
}

// A field over the cells at `p` (tank coordinates), trilinear between the cell centres; none
// outside.
float field_at(LiquidView* v, float* field, float3 p)
{
    // Air over the open top; at the glass and the floor the field goes on as it stands beside
    // them (as the smoothing takes it), so the surface's normal there does not turn into the
    // glass (it did: rays through a pane into thin water came out speckled).
    float3 size = v[0].size.xyz;
    if (p.y > size.y)
        return 0.0;
    p = clamp(p, float3(0.0), size);
    int3 n = int3(v[0].cells.xyz);
    float3 g = clamp(p / v[0].tank.w - 0.5, float3(0.0), float3(n - 1));
    int3 base = min(int3(floor(g)), n - 2);
    float3 t = g - float3(base);
    float r = 0.0;
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        float3 w3 = select(o == 1, t, 1.0 - t);
        r += w3.x * w3.y * w3.z * field[cell_index(v, base + o)];
    }
    return r;
}

// The density at `p` (tank coordinates), trilinear between the cell centres; none outside.
float density_at(LiquidView* v, float3 p)
{
    // Air over the open top; at the glass and the floor the field goes on as it stands beside
    // them (as the smoothing takes it), so the surface's normal there does not turn into the
    // glass (it did: rays through a pane into thin water came out speckled).
    float3 size = v[0].size.xyz;
    if (p.y > size.y)
        return 0.0;
    p = clamp(p, float3(0.0), size);
    int3 n = int3(v[0].cells.xyz);
    float3 g = clamp(p / v[0].tank.w - 0.5, float3(0.0), float3(n - 1));
    int3 base = min(int3(floor(g)), n - 2);
    float3 t = g - float3(base);
    float r = 0.0;
    for (uint k = 0u; k < 8u; k++)
    {
        int3 o = int3(int(k & 1u), int((k >> 1u) & 1u), int((k >> 2u) & 1u));
        float3 w3 = select(o == 1, t, 1.0 - t);
        r += w3.x * w3.y * w3.z * v[0].render[cell_index(v, base + o)];
    }
    return r;
}

// The surface's normal at `p`, out of the water.
float3 normal_at(LiquidView* v, float3 p)
{
    float e = v[0].tank.w;
    float3 g = float3(density_at(v, p + float3(e, 0, 0)) - density_at(v, p - float3(e, 0, 0)),
                      density_at(v, p + float3(0, e, 0)) - density_at(v, p - float3(0, e, 0)),
                      density_at(v, p + float3(0, 0, e)) - density_at(v, p - float3(0, 0, e)));
    float l = length(g);
    return l > 1e-6 ? -g / l : float3(0.0, 1.0, 0.0);
}

// Where the ray `p + t d` leaves the box `lo`..`hi` (`p` inside it), and the face's outward normal.
float box_exit(float3 p, float3 d, float3 lo, float3 hi, out float3 face)
{
    float3 inv = 1.0 / d;
    float3 t = max((lo - p) * inv, (hi - p) * inv);
    float exit = min(t.x, min(t.y, t.z));
    face = exit == t.x ? float3(sign(d.x), 0, 0) : (exit == t.y ? float3(0, sign(d.y), 0) : float3(0, 0, sign(d.z)));
    return max(exit, 0.0);
}

// Where the ray enters and leaves the box (enter > leave: it misses), and the entry face's
// outward normal.
float2 box_span(float3 p, float3 d, float3 lo, float3 hi, out float3 face)
{
    float3 inv = 1.0 / d;
    float3 a = (lo - p) * inv;
    float3 b = (hi - p) * inv;
    float3 near = min(a, b);
    float3 far = max(a, b);
    float enter = max(near.x, max(near.y, near.z));
    float leave = min(far.x, min(far.y, far.z));
    face = enter == near.x ? float3(-sign(d.x), 0, 0) : (enter == near.y ? float3(0, -sign(d.y), 0) : float3(0, 0, -sign(d.z)));
    return float2(enter, leave);
}

// How far along the ray from `p` the density first crosses the surface, the ray starting in
// the water (`water`) or out of it; `span` if it does not within `span`. Adds the foam it
// passes through (its foam times metres) to `foam_path`.
float crossing(LiquidView* v, float3 p, float3 d, float span, bool water, inout uint steps, inout float foam_path)
{
    float h = v[0].tank.w;
    float step = 0.5 * h;
    float brick = h * float(BRICK);
    int3 bricks = int3(v[0].bricks.xyz);
    float s = 0.0;
    float last = 0.0;
    while (s < span && steps < MAX_STEPS)
    {
        steps++;
        float3 q = p + d * s;
        int3 k = clamp(int3(floor(q / brick)), int3(0), bricks - 1);
        float4 range = v[0].brick_range[uint(k.x) + uint(bricks.x) * (uint(k.y) + uint(bricks.y) * uint(k.z))];
        if (range.z < FOAM_EPS && ((water && range.x >= SURFACE) || (!water && range.y < SURFACE)))
        {
            // The brick is all water or all air, and no foam: on to its far side.
            float3 face;
            float3 lo = float3(k) * brick;
            last = s;
            s += box_exit(q, d, lo, lo + brick, face) + 0.01 * h;
            continue;
        }
        if (range.z >= FOAM_EPS)
            foam_path += field_at(v, v[0].foam_render, q) * step;
        float r = density_at(v, q);
        if ((r >= SURFACE) != water)
        {
            // Halve the step's interval five times.
            float a = last;
            float b = s;
            for (uint i = 0u; i < 5u; i++)
            {
                float m = 0.5 * (a + b);
                if ((density_at(v, p + d * m) >= SURFACE) != water)
                    b = m;
                else
                    a = m;
            }
            return b;
        }
        last = s;
        s += step;
    }
    return span;
}

float fresnel(float f0, float cosine)
{
    return f0 + (1.0 - f0) * pow(1.0 - saturate(cosine), 5.0);
}

// What a surface of normal `n` mirrors towards a ray going along `d`: the sky (kept above the
// horizon) and the sun's highlight.
float3 mirrored(LiquidView* v, float3 d, float3 n)
{
    float3 r = reflect(d, n);
    r.y = abs(r.y);
    r = normalize(r);
    float4 up_table = v[0].sky_light[SH_COEFFICIENTS];
    float3 forward = v[0].sky_light[SH_COEFFICIENTS + 1].xyz;
    float scale = v[0].sun_radiance.w;
    float3 sky = v[0].background.w > 0.0 ? v[0].background.rgb
                                         : sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, forward, r) * scale;
    float3 l = normalize(v[0].sun.xyz);
    float a2 = v[0].sun.w;
    float3 h = normalize(l - d);
    float nh = saturate(dot(n, h));
    float nl = dot(n, l);
    float sun = 0.0;
    if (nl > 0.0)
    {
        float k = nh * nh * (a2 - 1.0) + 1.0;
        sun = a2 / (PI * k * k) * nl * 0.25 / max(saturate(dot(n, -d)) * nl, 1e-3);
    }
    return sky + v[0].sun_radiance.rgb * min(sun, 400.0);
}

// The light the water scatters towards the camera per unit of its scattering: the sky's from
// above and the sun's, spread evenly.
float3 in_water_light(LiquidView* v)
{
    float scale = v[0].sun_radiance.w;
    float3 sky = v[0].background.w > 0.0 ? v[0].background.rgb
                                         : sh_irradiance(v[0].sky_light, float3(0.0, 1.0, 0.0)) * scale / PI;
    return 0.5 * sky + v[0].sun_radiance.rgb * saturate(v[0].sun.y) / (4.0 * PI);
}

// A speed over the view's scale (0 to 1) as a colour: deep blue, cyan, white, orange.
float3 speed_colour(float t)
{
    t = saturate(t);
    float3 a = float3(0.05, 0.15, 0.6);
    float3 b = float3(0.1, 0.7, 0.9);
    float3 c = float3(0.95, 0.95, 0.95);
    float3 d = float3(1.0, 0.45, 0.1);
    return t < 0.33 ? lerp(a, b, t / 0.33) : (t < 0.66 ? lerp(b, c, (t - 0.33) / 0.33) : lerp(c, d, (t - 0.66) / 0.34));
}

// What foam sends back: a white matte surface's light under the sky and the sun.
float3 foam_light(LiquidView* v)
{
    float scale = v[0].sun_radiance.w;
    float3 sky = v[0].background.w > 0.0 ? v[0].background.rgb
                                         : sh_irradiance(v[0].sky_light, float3(0.0, 1.0, 0.0)) * scale / PI;
    return 0.85 * (sky + v[0].sun_radiance.rgb * saturate(v[0].sun.y) / PI);
}

// What the scene shows where the ray from `o` (camera-relative) along `d` first goes behind it:
// marched over the screen against the depth (steps growing from a centimetre, 30 m in all), the
// crossing halved four times; where it leaves the screen or meets nothing, the last place seen.
float3 behind_along(LiquidView* v, float3 o, float3 d, uint2 pixel)
{
    float2 size = float2(v[0].width, v[0].height);
    float t = 0.0;
    float before = 0.0;
    float stride = 0.01;
    float2 seen = float2(-1.0);
    for (uint i = 0u; i < 24u; i++)
    {
        t += stride;
        stride *= 1.35;
        float4 clip = mul(v[0].view_proj, float4(o + d * t, 1.0));
        if (clip.w <= 1e-4 || any(abs(clip.xy) > clip.w))
            break;
        float2 uv = float2(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        seen = uv;
        float scene = g_sampled_images_r[v[0].depth].Load(int3(int2(uv * size), 0));
        // Behind what the scene shows there, by less than a hand (reversed-Z: the view depth is
        // the near plane, clip.z, over the depth): further behind, that is something in front
        // of the ray, not where it lands.
        float behind = clip.w - (scene > 0.0 ? clip.z / scene : 1e9);
        if (behind >= 0.0 && behind < THICKNESS)
        {
            float a = before;
            float b = t;
            for (uint k = 0u; k < 4u; k++)
            {
                float m = 0.5 * (a + b);
                float4 c = mul(v[0].view_proj, float4(o + d * m, 1.0));
                float2 u = float2(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
                if (c.z / c.w <= g_sampled_images_r[v[0].depth].Load(int3(int2(saturate(u) * size), 0)))
                {
                    b = m;
                    seen = u;
                }
                else
                    a = m;
            }
            break;
        }
        before = t;
    }
    if (seen.x < 0.0)
        return g_sampled_images[v[0].scene].Load(int3(pixel, 0)).rgb;
    return sample_image_level(v[0].scene, SAMPLER_LINEAR_CLAMP, saturate(seen), 0.0).rgb;
}

float3 face_velocity(LiquidView* v, float3 p)
{
    int3 n = int3(v[0].cells.xyz);
    int3 c = clamp(int3(floor(p / v[0].tank.w)), int3(0), n - 1);
    uint i = uint(c.x) + uint(n.x + 1) * (uint(c.y) + uint(n.y + 1) * uint(c.z));
    return float3(v[0].face[i], v[0].face[v[0].nodes + i], v[0].face[2u * v[0].nodes + i]);
}

// Where the depth is empty, the plain background, if there is one.
void plain_background(LiquidView* v, uint2 pixel, float depth)
{
    if (v[0].background.w > 0.0 && depth <= 0.0)
        g_storage_images_rgba16f[v[0].color][pixel] = float4(v[0].background.rgb, 1.0);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void march_main(uint3 id : SV_DispatchThreadID)
{
    LiquidView* v = view_push.view;
    if (id.x >= v[0].width || id.y >= v[0].height)
        return;
    float2 pixel = float2(id.xy) + 0.5;
    float2 uv = pixel / float2(v[0].width, v[0].height);
    float2 ndc = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    float4 far = mul(v[0].inv_view_proj, float4(ndc, 0.5, 1.0));
    float3 d = normalize(far.xyz / far.w);
    float depth = g_sampled_images_r[v[0].depth].Load(int3(id.xy, 0));
    float scene = 1e9;
    if (depth > 0.0)
    {
        float4 hit = mul(v[0].inv_view_proj, float4(ndc, depth, 1.0));
        scene = length(hit.xyz / hit.w);
    }
    float3 size = v[0].size.xyz;
    float g = v[0].size.w;
    // The camera in the tank's frame.
    float3 o = -v[0].tank.xyz;
    // The glass box, open at the top.
    float3 entry_face;
    float2 outer = box_span(o, d, float3(-g, -g, -g), float3(size.x + g, size.y, size.z + g), entry_face);
    if (outer.x >= outer.y || outer.y <= 0.0 || max(outer.x, 0.0) >= scene)
    {
        plain_background(v, id.xy, depth);
        return;
    }
    float3 inner_face;
    float2 inner = box_span(o, d, float3(0.0), size, inner_face);
    float start = max(inner.x, 0.0);
    if (inner.x >= inner.y || inner.y <= 0.0 || start >= scene)
    {
        plain_background(v, id.xy, depth);
        return;
    }
    float3 sigma = v[0].absorption.rgb + v[0].scattering.rgb;
    float3 albedo = v[0].scattering.rgb / max(sigma, 1e-6);
    float3 lit = in_water_light(v);
    float3 foam_lit = foam_light(v);
    float ior = v[0].absorption.w;
    float3 colour = 0.0;
    float3 through = 1.0;
    // Through a pane on the way in (not over the open top, nor from inside the glass box).
    bool shows = v[0].glass.w > 0.0;
    bool pane_in = shows && outer.x > 0.0 && entry_face.y < 0.5;
    if (pane_in)
    {
        float f = fresnel(GLASS_F0, dot(-d, entry_face));
        colour += f * mirrored(v, d, entry_face);
        through *= (1.0 - f) * v[0].glass.rgb;
    }
    float3 p = o + d * start;
    float3 dir = d;
    float travelled = start;
    float left = min(inner.y, scene) - start;
    bool water = density_at(v, p + dir * (0.01 * v[0].tank.w)) >= SURFACE;
    // The speed view, the water met at the tank's side: its colour there.
    if (v[0].mode == 1u && water)
    {
        float speed = length(face_velocity(v, p));
        colour += through * speed_colour(speed / SPEED_TOP) * foam_lit * 0.6;
        through = 0.0;
    }
    // Entering the water through a pane: bent at the glass as from air into water (flat glass
    // only shifts the ray).
    if (water && start > 0.0 && inner_face.y < 0.5)
    {
        float3 bent = refract(dir, inner_face, 1.0 / ior);
        if (dot(bent, bent) > 0.0)
            dir = normalize(bent);
    }
    bool bent_any = water && start > 0.0 && inner_face.y < 0.5;
    float reactive = 0.0;
    uint steps = 0u;
    for (uint crossings = 0u; crossings < 8u; crossings++)
    {
        float3 face;
        float to_box = box_exit(p, dir, float3(0.0), size, face);
        // What the scene shows inside the tank (the gate) ends the ray; once bent, only roughly.
        float limit = bent_any ? max(scene - travelled, 0.0) + 0.5 : left;
        float span = min(to_box, limit);
        float foam_path = 0.0;
        float s = crossing(v, p, dir, span, water, steps, foam_path);
        if (water)
        {
            float3 t = exp(-sigma * s);
            colour += through * albedo * (1.0 - t) * lit;
            through *= t;
        }
        // Foam on the way, in the water or in the spray: white, lit as a matte surface.
        if (foam_path > 0.0)
        {
            float t = exp(-FOAM_SIGMA * foam_path);
            colour += through * (1.0 - t) * foam_lit;
            through *= t;
        }
        p += dir * s;
        travelled += s;
        left -= s;
        if (s >= span)
        {
            // At a pane (not the open top): from the air straight through; from the water out,
            // bent as from water into air (flat glass only shifts the ray), or mirrored whole
            // past the critical angle and on through the water. The floor's glass lies on the
            // table: no air under it to mirror, the table seen through it.
            if (to_box > limit || face.y > 0.5)
                break;
            if (water && face.y > -0.5)
            {
                float3 bent = refract(dir, -face, ior);
                bent_any = true;
                if (dot(bent, bent) == 0.0)
                {
                    dir = reflect(dir, face);
                    p += dir * (0.02 * v[0].tank.w);
                    continue;
                }
                through *= 1.0 - fresnel(WATER_F0, dot(dir, face));
                dir = normalize(bent);
            }
            if (shows)
                through *= v[0].glass.rgb * (1.0 - GLASS_F0);
            break;
        }
        float3 n = normal_at(v, p);
        if (!water)
        {
            if (v[0].mode == 1u)
            {
                // The speed view (for tuning, after Sebastian Lague's renders): the surface matte,
                // blue when still, through cyan and white to orange at SPEED_TOP.
                float speed = length(face_velocity(v, p));
                colour += through * speed_colour(speed / SPEED_TOP) * foam_lit * (0.35 + 0.65 * saturate(dot(n, normalize(v[0].sun.xyz))));
                through = 0.0;
                break;
            }
            // Into the water: Fresnel's reflection, the rest bent down into it.
            float f = fresnel(WATER_F0, dot(-dir, n));
            colour += through * f * mirrored(v, dir, n);
            through *= 1.0 - f;
            float3 bent = refract(dir, n, 1.0 / ior);
            dir = dot(bent, bent) > 0.0 ? normalize(bent) : dir;
            water = true;
            float speed = length(face_velocity(v, p));
            reactive = max(reactive, REACTIVE * saturate(speed / REACTIVE_SPEED));
        }
        else
        {
            // Out through the surface from below, or mirrored whole past the critical angle.
            float3 bent = refract(dir, -n, ior);
            if (dot(bent, bent) == 0.0)
                dir = reflect(dir, -n);
            else
            {
                through *= 1.0 - fresnel(WATER_F0, dot(dir, n));
                dir = normalize(bent);
                water = false;
            }
        }
        bent_any = true;
        p += dir * (0.02 * v[0].tank.w);
    }
    float3 behind = bent_any ? behind_along(v, p + v[0].tank.xyz, dir, id.xy)
                             : g_sampled_images[v[0].scene].Load(int3(id.xy, 0)).rgb;
    float3 result = colour + through * behind;
    g_storage_images_rgba16f[v[0].color][id.xy] = float4(result, 1.0);
    if (v[0].reactive != 0xFFFFFFFFu)
        g_storage_images_r32f[v[0].reactive][id.xy] = reactive;
}

struct ClearPush
{
    uint image;
    uint width;
    uint height;
    uint pad;
};

[[vk::push_constant]]
ClearPush clear_push;

[shader("compute")]
[numthreads(8, 8, 1)]
void clear_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= clear_push.width || id.y >= clear_push.height)
        return;
    g_storage_images_r32f[clear_push.image][id.xy] = 0.0;
}
