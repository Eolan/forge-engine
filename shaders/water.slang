// The sea's waves on the GPU (issue #105, D-038; `crates/forge-render/src/water.rs`): each
// cascade's spectrum, evolved to this frame's time and turned into a tiling patch of
// displacements, slopes and the Jacobian by an inverse FFT (Tessendorf 2001), four passes a
// cascade on the async compute queue:
// - `water/evolve`: h(k, t) = h0(k) e^{iωt} + conj(h0(−k)) e^{−iωt}, and from it the spectra of
//   the height, the two horizontal displacements, the two slopes and the displacement's three
//   derivatives, packed two real fields to a complex one (their spectra are Hermitian, so the
//   transform of A + iB is a + ib);
// - `water/fft-rows` and `water/fft-cols`: the inverse transform of every row, then every
//   column, a workgroup each, radix-2 Stockham in groupshared memory (Govindaraju et al. 2008);
// - `water/derive`: the fields unpacked into two images, the Jacobian from the derivatives.
// The spectrum is the CPU's (`forge_procgen::Ocean::gpu_samples`), and the transform is the
// CPU's `ifft2` in single precision: the CPU's `Ocean::surface` is the reference.
import bindless;
import atmosphere;
import sh;
import skyframe;
import skyview;

// Samples per side of a cascade (`WATER_SIZE` in water.rs).
static const uint N = 256;
static const uint LOG_N = 8;
// Packed complex fields per sample.
static const uint FIELDS = 4;
static const float PI = 3.14159265358979;

struct WaterPush
{
    float4* spectrum;  // per sample: h0 (re, im), ω, 0 (`Ocean::gpu_samples`)
    float2* work;      // per sample, FIELDS complex values: the packed spectra, then the fields
    float time;        // seconds
    float patch;       // metres a side
    float choppiness;  // the horizontal displacement's scale
    uint displacement; // storage index (rgba16f): x, height, y (world x, y, z), 0
    uint slopes;       // storage index (rgba16f): ∂h/∂x, ∂h/∂y, the Jacobian, 0
    uint pad0;
    uint2 pad1;
};

[[vk::push_constant]]
WaterPush push;

float2 cmul(float2 a, float2 b)
{
    return float2(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

// i · a.
float2 times_i(float2 a)
{
    return float2(-a.y, a.x);
}

// ------------------------------------------------------------------ evolve ---

[shader("compute")]
[numthreads(16, 16, 1)]
void evolve_main(uint3 id : SV_DispatchThreadID)
{
    uint x = id.x, y = id.y;
    uint i = y * N + x;
    uint mx = (N - x) % N, my = (N - y) % N;
    uint j = my * N + mx; // −k
    float4 s = push.spectrum[i];
    float4 t = push.spectrum[j];
    float phi = s.z * push.time;
    float2 phase = float2(cos(phi), sin(phi));
    float2 h = cmul(s.xy, phase) + cmul(float2(t.x, -t.y), float2(phase.x, -phase.y));
    // The wave vector, frequencies in FFT order (0..N/2 positive, then negative).
    float dk = 2.0 * PI / push.patch;
    float kx = float(int(x < N / 2 ? x : x - N)) * dk;
    float ky = float(int(y < N / 2 ? y : y - N)) * dk;
    float kk = sqrt(kx * kx + ky * ky);
    float ux = kk > 0.0 ? kx / kk : 0.0;
    float uy = kk > 0.0 ? ky / kk : 0.0;
    float lambda = push.choppiness;
    // The CPU's fields (`Ocean::surface`): displacement −i k̂ h, slopes i k h, the
    // displacement's derivatives k̂ k h.
    float2 height = h;
    float2 dx = float2(h.y * ux, -h.x * ux) * lambda;
    float2 dy = float2(h.y * uy, -h.x * uy) * lambda;
    float2 sx = float2(-h.y * kx, h.x * kx);
    float2 sy = float2(-h.y * ky, h.x * ky);
    float2 dxx = h * (ux * kx * lambda);
    float2 dyy = h * (uy * ky * lambda);
    float2 dxy = h * (ux * ky * lambda);
    if (i == j)
    {
        // A wave vector that is its own opposite (0 or N/2 on each axis): the CPU keeps the
        // real part of each field's transform, which is the real part of its coefficient here.
        height.y = 0.0;
        dx.y = 0.0;
        dy.y = 0.0;
        sx.y = 0.0;
        sy.y = 0.0;
        dxx.y = 0.0;
        dyy.y = 0.0;
        dxy.y = 0.0;
    }
    uint w = i * FIELDS;
    push.work[w + 0] = height + times_i(dx);
    push.work[w + 1] = dy + times_i(sx);
    push.work[w + 2] = sy + times_i(dxx);
    push.work[w + 3] = dyy + times_i(dxy);
}

// --------------------------------------------------------------------- FFT ---

// Two buffers (the stages ping-pong) of FIELDS rows of N: 16 KB.
groupshared float2 g_fft[2 * FIELDS * N];

uint fft_at(uint buffer, uint field, uint index)
{
    return (buffer * FIELDS + field) * N + index;
}

// The inverse transform of the FIELDS lines in `g_fft`'s first buffer, in place (the result
// ends in the first buffer: LOG_N is even). Stockham's radix-2 autosort: stage s combines
// pairs `Ns = 2^s` apart with the twiddle e^{+iπk/Ns}, unnormalised, as the CPU's `ifft`.
void fft_lines(uint thread)
{
    uint j = thread % (N / 2);  // the butterfly
    uint first = thread / (N / 2); // fields `first` and `first + 2`
    uint source = 0;
    for (uint s = 0; s < LOG_N; s++)
    {
        uint ns = 1u << s;
        uint k = j & (ns - 1u);
        float angle = PI * float(k) / float(ns);
        float2 w = float2(cos(angle), sin(angle));
        uint to = (j / ns) * ns * 2u + k;
        for (uint f = first; f < FIELDS; f += 2u)
        {
            float2 a = g_fft[fft_at(source, f, j)];
            float2 b = cmul(g_fft[fft_at(source, f, j + N / 2)], w);
            g_fft[fft_at(1u - source, f, to)] = a + b;
            g_fft[fft_at(1u - source, f, to + ns)] = a - b;
        }
        source = 1u - source;
        GroupMemoryBarrierWithGroupSync();
    }
}

[shader("compute")]
[numthreads(N, 1, 1)]
void fft_rows_main(uint3 group : SV_GroupID, uint thread : SV_GroupIndex)
{
    uint row = group.x;
    for (uint f = 0; f < FIELDS; f++)
        g_fft[fft_at(0, f, thread)] = push.work[(row * N + thread) * FIELDS + f];
    GroupMemoryBarrierWithGroupSync();
    fft_lines(thread);
    for (uint f = 0; f < FIELDS; f++)
        push.work[(row * N + thread) * FIELDS + f] = g_fft[fft_at(0, f, thread)];
}

[shader("compute")]
[numthreads(N, 1, 1)]
void fft_cols_main(uint3 group : SV_GroupID, uint thread : SV_GroupIndex)
{
    uint column = group.x;
    for (uint f = 0; f < FIELDS; f++)
        g_fft[fft_at(0, f, thread)] = push.work[(thread * N + column) * FIELDS + f];
    GroupMemoryBarrierWithGroupSync();
    fft_lines(thread);
    for (uint f = 0; f < FIELDS; f++)
        push.work[(thread * N + column) * FIELDS + f] = g_fft[fft_at(0, f, thread)];
}

// ------------------------------------------------------------------ derive ---

[shader("compute")]
[numthreads(16, 16, 1)]
void derive_main(uint3 id : SV_DispatchThreadID)
{
    uint w = (id.y * N + id.x) * FIELDS;
    float2 p0 = push.work[w + 0];
    float2 p1 = push.work[w + 1];
    float2 p2 = push.work[w + 2];
    float2 p3 = push.work[w + 3];
    // height, dx | dy, ∂h/∂x | ∂h/∂y, ∂dx/∂x | ∂dy/∂y, ∂dx/∂y
    float jacobian = (1.0 + p2.y) * (1.0 + p3.x) - p3.y * p3.y;
    g_storage_images_rgba16f[push.displacement][id.xy] = float4(p0.y, p0.x, p1.x, 0.0);
    // The slopes' squared length too: its mean over a mip's texels, less the mean slope's,
    // is the variance of the slopes that mip no longer resolves (Bruneton, Neyret &
    // Holzschuch 2010), the roughness the surface adds for them.
    g_storage_images_rgba16f[push.slopes][id.xy] = float4(p1.y, p2.x, jacobian, p1.y * p1.y + p2.x * p2.x);
}

// -------------------------------------------------------------------- mips ---

struct MipsPush
{
    uint displacement; // sampled indices (every level)
    uint slopes;
    uint displacement_out; // storage indices of level `level`
    uint slopes_out;
    uint level;
    uint size;         // texels a side at `level`
    uint2 pad;
};

[[vk::push_constant]]
MipsPush mips_push;

// Level `level` of both images from the level below: the mean of each 2 × 2.
[shader("compute")]
[numthreads(8, 8, 1)]
void mips_main(uint3 id : SV_DispatchThreadID)
{
    MipsPush p = mips_push;
    if (id.x >= p.size || id.y >= p.size)
        return;
    int source = int(p.level) - 1;
    Texture2D<float4> displacement = g_sampled_images[p.displacement];
    Texture2D<float4> slopes = g_sampled_images[p.slopes];
    int2 at = int2(id.xy) * 2;
    float4 d = 0.0, s = 0.0;
    for (int y = 0; y < 2; y++)
        for (int x = 0; x < 2; x++)
        {
            d += displacement.Load(int3(at + int2(x, y), source));
            s += slopes.Load(int3(at + int2(x, y), source));
        }
    g_storage_images_rgba16f[p.displacement_out][id.xy] = d * 0.25;
    g_storage_images_rgba16f[p.slopes_out][id.xy] = s * 0.25;
}

// ---------------------------------------------------------------- surface ---

// The sea's surface (issue #105, step 2): a forward pass after the sky's compose. The mesh is
// a clipmap of square grids around the camera (Losasso & Hoppe 2004), `grid` quads a side,
// each level twice as coarse as the one before and centred on the camera snapped to twice its
// spacing, leaving out the quads the finer level covers. Near its outer edge a level's odd
// vertices slide onto the coarser lattice, all the way by the edge, so the levels meet
// without cracks. The cascades displace it, each filtered to the vertex spacing by its mips;
// the fragment takes its normal and the variance of the slopes it no longer resolves from
// the slopes' mips, reflects the sky, adds the sun's highlight, and lets through what lies
// under the water, dimmed by the depth it crosses.

static const uint WATER_CASCADES = 3;
static const uint WATER_MAX_LEVELS = 16;

// Mirrors `GpuWaterCascadeView` in water.rs.
struct WaterCascadeView
{
    float2 offset;     // the camera's place in the patch, as a share of it
    float inv_patch;   // 1 / the patch's side, m⁻¹
    float texel;       // metres a texel
    uint displacement; // sampled indices (every mip)
    uint slopes;
    uint2 pad;
};

// Mirrors `GpuWaterSurface` in water.rs.
struct WaterSurface
{
    float4x4 view_proj;   // camera-relative metres → clip (the drawing camera, jitter included)
    float4 camera;        // xyz: the camera's world position, its height over the sea in y
    float4 sun;           // xyz: towards the sun
    float4 sun_radiance;  // rgb: the sun's pre-exposed illuminance on a surface facing it; w: the sky's luminance scale
    float4 absorption;    // rgb: the water's absorption, m⁻¹
    float4 scatter;       // rgb: the light the water scatters back, per unit of irradiance
    WaterCascadeView cascades[WATER_CASCADES];
    float4 levels[WATER_MAX_LEVELS]; // per level: its centre relative to the camera (xy = world xz), its spacing (z)
    uint level_count;
    uint grid;            // quads a side of a level, even
    uint scene_color;     // sampled: the HDR image under the water
    uint scene_depth;     // sampled: its depth (r32f)
    uint width;
    uint height;
    uint2 pad;
    Sky* sky;             // the ground sky's block (the aerial perspective)
    float4* sky_light;    // the sky's irradiance, then the sky-view table's frame (sky.slang)
};

struct SurfacePush
{
    WaterSurface* surface;
    uint2 pad;
};

[[vk::push_constant]]
SurfacePush surface_push;

struct SurfaceOut
{
    float4 position : SV_Position;
    float3 relative : POSITION;   // the displaced point, camera-relative metres
    float2 flat : TEXCOORD0;      // the undisplaced point, camera-relative (world x, z)
};

// A point's displacement from every cascade, each sampled at the mip whose texels match
// `spacing` metres.
float3 displacement_at(WaterSurface* w, float2 flat, float spacing)
{
    float3 d = 0.0;
    for (uint c = 0; c < WATER_CASCADES; c++)
    {
        WaterCascadeView v = w[0].cascades[c];
        float2 uv = flat * v.inv_patch + v.offset;
        float mip = log2(max(spacing / v.texel, 1.0));
        d += sample_image_level(v.displacement, SAMPLER_LINEAR_REPEAT, uv, mip).xyz;
    }
    return d;
}

// Two triangles a quad, counter-clockwise seen from above.
static const int2 QUAD_CORNERS[6] = { int2(0, 0), int2(0, 1), int2(1, 0), int2(1, 0), int2(0, 1), int2(1, 1) };

[shader("vertex")]
SurfaceOut surface_vert_main(uint id : SV_VertexID)
{
    WaterSurface* w = surface_push.surface;
    uint g = w[0].grid;
    uint quad = id / 6u;
    uint corner = id % 6u;
    uint level = quad / (g * g);
    uint q = quad % (g * g);
    int2 cell = int2(int(q % g), int(q / g));
    int half_grid = int(g / 2u);
    int2 lattice = cell + QUAD_CORNERS[corner] - half_grid; // from the level's centre, in spacings
    float4 l = w[0].levels[level];
    float s = l.z;
    float2 centre = l.xy;
    SurfaceOut o;
    // The quads the finer level covers: its extent, half as wide around its own centre.
    if (level > 0u)
    {
        float2 fine = w[0].levels[level - 1u].xy;
        float2 middle = centre + (float2(cell) + 0.5 - float(half_grid)) * s;
        float half_fine = float(half_grid) * s * 0.5;
        if (all(abs(middle - fine) < half_fine))
        {
            o.position = float4(0.0, 0.0, 0.0, 0.0); // degenerate: nothing drawn
            o.relative = 0.0;
            o.flat = 0.0;
            return o;
        }
    }
    float2 flat = centre + float2(lattice) * s;
    // The slide onto the coarser lattice: complete by where the level's edge can come
    // nearest the camera (its half width less the two spacings the snapping moves it).
    float half_width = float(half_grid) * s;
    float band = 0.25 * half_width;
    float nearest_edge = half_width - 2.0 * s;
    float reach = max(abs(flat.x), abs(flat.y));
    float morph = level + 1u < w[0].level_count ? saturate((reach - (nearest_edge - band)) / band) : 0.0;
    int2 odd = lattice & 1;
    flat -= float2(odd) * s * morph;
    float3 d = displacement_at(w, flat, s * (1.0 + morph));
    float3 relative = float3(flat.x + d.x, d.y - w[0].camera.y, flat.y + d.z);
    o.position = mul(w[0].view_proj, float4(relative, 1.0));
    o.relative = relative;
    o.flat = flat;
    return o;
}

// GGX's distribution for the half vector's cosine `nh` and roughness² `a2`.
float ggx(float nh, float a2)
{
    float d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Smith's masking for GGX, both directions (the height-correlated form, over 4 n·v n·l).
float smith_visibility(float nv, float nl, float a2)
{
    float v = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    float l = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(v + l, 1e-6);
}

[shader("fragment")]
float4 surface_frag_main(SurfaceOut input) : SV_Target
{
    WaterSurface* w = surface_push.surface;
    // The slopes: their mean from every cascade's mips, and the variance of those the mips
    // average away (Bruneton, Neyret & Holzschuch 2010).
    float2 slope = 0.0;
    float variance = 0.0;
    float jacobian = 1.0;
    for (uint c = 0; c < WATER_CASCADES; c++)
    {
        WaterCascadeView cv = w[0].cascades[c];
        float2 uv = input.flat * cv.inv_patch + cv.offset;
        float4 s = sample_image(cv.slopes, SAMPLER_ANISOTROPIC_REPEAT, uv);
        slope += s.xy;
        variance += max(s.w - dot(s.xy, s.xy), 0.0);
        jacobian += s.z - 1.0;
    }
    float3 n = normalize(float3(-slope.x, 1.0, -slope.y));
    float3 v = normalize(-input.relative);
    float nv = max(dot(n, v), 1e-4);
    // GGX's α² from the unresolved slopes' variance (α² = 2σ²), over a calm surface's.
    float a2 = clamp(0.0004 + 2.0 * variance, 0.0004, 1.0);
    float f0 = 0.02;
    float fresnel = f0 + (1.0 - f0) * pow(1.0 - nv, 5.0);
    float4 up_table = w[0].sky_light[SH_COEFFICIENTS];
    float3 up = up_table.xyz;
    float3 forward = w[0].sky_light[SH_COEFFICIENTS + 1].xyz;
    float scale = w[0].sun_radiance.w;
    // The sky in the mirror direction (kept above the horizon: a wave's back cannot reflect
    // the sea), blurred towards the sky's irradiance as the surface roughens.
    float3 r = reflect(-v, n);
    r.y = abs(r.y);
    float3 mirror = sky_view_luminance(uint(up_table.w + 0.5), up, forward, normalize(r)) * scale;
    float3 sky_irradiance = sh_irradiance(w[0].sky_light, float3(0.0, 1.0, 0.0)) * scale / PI;
    float3 reflected = lerp(mirror, sky_irradiance, saturate(2.0 * a2));
    // The sun's highlight.
    float3 l = normalize(w[0].sun.xyz);
    float nl = dot(n, l);
    float3 sun = 0.0;
    if (nl > 0.0)
    {
        float3 h = normalize(l + v);
        float fh = f0 + (1.0 - f0) * pow(1.0 - saturate(dot(v, h)), 5.0);
        sun = w[0].sun_radiance.rgb * ggx(saturate(dot(n, h)), a2) * smith_visibility(nv, nl, a2) * fh * nl;
    }
    // What lies under the water: the scene behind this pixel, dimmed along the path the view
    // ray takes through the water to it, and the light the water scatters back on the way.
    uint2 pixel = uint2(input.position.xy);
    float2 uv = (float2(pixel) + 0.5) / float2(w[0].width, w[0].height);
    float floor_depth = g_sampled_images_r[NonUniformResourceIndex(w[0].scene_depth)].Load(int3(pixel, 0));
    float surface_distance = length(input.relative);
    float path = 1e6;
    if (floor_depth > 0.0)
    {
        float4 behind = mul(w[0].sky[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, floor_depth, 1.0));
        path = max(length(behind.xyz / behind.w) - surface_distance, 0.0);
    }
    float3 transmittance = exp(-w[0].absorption.rgb * path);
    float3 under = g_sampled_images[NonUniformResourceIndex(w[0].scene_color)].Load(int3(pixel, 0)).rgb;
    float3 lit = sky_irradiance * PI + w[0].sun_radiance.rgb * saturate(l.y);
    float3 inside = under * transmittance + w[0].scatter.rgb * lit * (1.0 - transmittance);
    float3 color = reflected * fresnel + inside * (1.0 - fresnel) + sun;
    // Foam where the waves fold (the Jacobian under ~0.45), lit like a white diffuse surface.
    float foam = saturate((0.45 - jacobian) * 2.0);
    color = lerp(color, 0.8 * lit / PI, foam * 0.8);
    // The air between the camera and the water, as the compose hazes everything else.
    float4 air = aerial_at(w[0].sky, uv, surface_distance * 0.001);
    color = color * air.a + air.rgb * scale;
    return float4(min(color, 32768.0), 1.0);
}

// ------------------------------------------------------------- scene copy ---

struct CopyPush
{
    uint color;      // sampled: the HDR image
    uint depth;      // sampled: the depth
    uint color_out;  // storage: the copies
    uint depth_out;
    uint width;
    uint height;
    uint2 pad;
};

[[vk::push_constant]]
CopyPush copy_push;

// What the water lets through, copied before it is drawn over: the HDR image and its depth.
[shader("compute")]
[numthreads(8, 8, 1)]
void copy_main(uint3 id : SV_DispatchThreadID)
{
    CopyPush p = copy_push;
    if (id.x >= p.width || id.y >= p.height)
        return;
    int3 at = int3(id.xy, 0);
    g_storage_images_rgba16f[p.color_out][id.xy] = g_sampled_images[p.color].Load(at);
    g_storage_images_r32f[p.depth_out][id.xy] = g_sampled_images_r[p.depth].Load(at);
}
