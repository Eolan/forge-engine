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
import shore;

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
// under the water, dimmed by the depth it crosses. It also asks for the mirror ray that puts
// the scene in the reflection (step 3) and the shadow ray that takes the sun away where the
// scene shades the water (step 4; `requested_rays_main` in meshlet.slang).

static const uint WATER_CASCADES = 3;
static const uint WATER_MAX_LEVELS = 16;
static const uint WATER_MAX_TRAINS = 4;

// Mirrors `GpuWaterCascadeView` in water.rs.
struct WaterCascadeView
{
    float2 offset;     // the camera's place in the patch, as a share of it
    float inv_patch;   // 1 / the patch's side, m⁻¹
    float texel;       // metres a texel
    uint displacement; // sampled indices (every mip)
    uint slopes;
    float omega;       // the angular frequency its energy centres on, rad/s
    float shelf;       // the TMA factor there at the depth the spectrum was made for
};

// A point of a river's ribbon (`GpuRiverPoint` in water.rs, from `forge_procgen::RibbonPoint`).
struct RiverPoint
{
    float4 a; // world x, z (the sea's frame), the half width (m), the depth in the middle (m)
    float4 b; // downstream (world x, z; unit), the speed (m/s), the bed's slope
    float4 c; // how much of the river is drawn (0..1), 1 where a segment starts (0 at a river's last point), the ground under the last vertex across, 0
    float4 d; // the ground under the first four vertices across: the highest the drawn ground stands under the quads around each (m)
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
    float4 shore_frame;   // the shore's first sample (world x, z) and 1 / its spacing
    float4 trains[WATER_MAX_TRAINS]; // per shore train: ω (rad/s), its height in deep water (m), a phase offset, 0
    WaterCascadeView cascades[WATER_CASCADES];
    float4 levels[WATER_MAX_LEVELS]; // per level: its centre relative to the camera (xy = world xz), its spacing (z)
    uint level_count;
    uint grid;            // quads a side of a level, even
    uint scene_color;     // sampled: the HDR image under the water
    uint scene_depth;     // sampled: its depth (r32f)
    uint width;
    uint height;
    uint shore;           // sampled (rg16f): the floor's height, the signed coast distance; or ~0: deep everywhere
    uint shore_texels;    // its samples a side
    Sky* sky;             // the ground sky's block (the aerial perspective)
    float4* sky_light;    // the sky's irradiance, then the sky-view table's frame (sky.slang)
    float2* shore_table;  // per train, `shore_bins` entries out from the shore: seconds to the shore, the shoaling coefficient
    uint shore_bins;
    uint train_count;
    float shore_bin;      // metres a bin
    float time;           // the sea's clock, seconds
    float pixel;          // metres a pixel spans a metre away
    uint river_points;
    float* ground;        // the floor's heights (the shore's samples) in full precision, or null: no rivers
    RiverPoint* rivers;   // `river_points` of them, one river after the other
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

// ----------------------------------------------------------------- the shore ---
//
// The waves feel the floor (step 5, D-038's shore): each cascade is damped by the root of the
// TMA factor of Kitaigorodskii (Bouws et al. 1985) at the frequency its energy centres on, in
// the water's depth, relative to the depth its spectrum was made for. Long waves die first in
// the shallows, and every cascade is still at the waterline, where the factor reaches zero; the
// open sea keeps its waves.

static const float GRAVITY = 9.81;
static const uint NO_SHORE = 0xFFFFFFFFu;

// The floor's height and the signed coast distance (metres, positive inland) under a point
// `flat` (camera-relative world x, z). Without the fields, a deep sea everywhere.
float2 shore_at(WaterSurface* w, float2 flat)
{
    if (w[0].shore == NO_SHORE)
        return float2(-1.0e4, -1.0e4);
    float2 world = flat + w[0].camera.xz;
    float2 uv = ((world - w[0].shore_frame.xy) * w[0].shore_frame.z + 0.5) / float(w[0].shore_texels);
    return sample_image_level(w[0].shore, SAMPLER_LINEAR_CLAMP, uv, 0.0).xy;
}

// Kitaigorodskii's factor at angular frequency `omega` in `depth` metres of water
// (`forge_procgen::tma`).
float tma(float omega, float depth)
{
    float wh = omega * sqrt(max(depth, 0.0) / GRAVITY);
    if (wh <= 1.0)
        return 0.5 * wh * wh;
    if (wh < 2.0)
        return 1.0 - 0.5 * (2.0 - wh) * (2.0 - wh);
    return 1.0;
}

// How much of cascade `v`'s waves stand in `depth` metres of water: 1 at the spectrum's own
// depth and deeper, 0 on land.
float shore_damping(WaterCascadeView v, float depth)
{
    if (depth <= 0.0 || v.shelf <= 0.0)
        return 0.0;
    return min(sqrt(tma(v.omega, depth) / v.shelf), 1.0);
}

// A point's displacement from every cascade, each sampled at the mip whose texels match
// `spacing` metres and damped by the `depth` under the point.
float3 displacement_at(WaterSurface* w, float2 flat, float spacing, float depth)
{
    float3 d = 0.0;
    for (uint c = 0; c < WATER_CASCADES; c++)
    {
        WaterCascadeView v = w[0].cascades[c];
        float2 uv = flat * v.inv_patch + v.offset;
        float mip = log2(max(spacing / v.texel, 1.0));
        d += shore_damping(v, depth) * sample_image_level(v.displacement, SAMPLER_LINEAR_REPEAT, uv, mip).xyz;
    }
    return d;
}

// ---------------------------------------------------------- the shore's waves ---
//
// Trains of waves come in along the coast distance's gradient (Uncharted 3's recipe, Gonzalez-
// Ochoa & Holder 2012; D-038's shore): each a Gerstner wave whose phase is ω (t + τ), τ the
// time its crests take from the point to the shore (`forge_procgen::ShoreTrain::table`, over
// the floor's profile), so they slow and bunch up over the shallows. Their height grows by the
// table's shoaling coefficient until the trains together stand higher than 0.78 of the depth
// (the breaker index), then follows the depth down: there they break, and the broken water is
// foam. At the shoreline the last of each wave runs up the beach as a thin sheet (the swash).
// The trains fade in where the floor starts to matter (the cascades fade out there), and out
// where they are too short for what is drawn (the mesh's spacing, the pixel's footprint).

static const float BREAKER_INDEX = 0.78;

// The shore under a point: the floor's depth and the coast distance.
struct ShorePoint
{
    float depth;    // metres (negative on land)
    float distance; // metres, positive inland
};

ShorePoint shore_point(WaterSurface* w, float2 flat)
{
    float2 s = shore_at(w, flat);
    ShorePoint p;
    p.depth = -s.x;
    p.distance = s.y;
    return p;
}

// The direction towards the shore at a point: the coast distance's gradient over a sample
// either side, unit (world x and z).
float2 shore_inland(WaterSurface* w, float2 flat)
{
    float e = 1.0 / w[0].shore_frame.z;
    float2 g = float2(
        shore_at(w, flat + float2(e, 0.0)).y - shore_at(w, flat - float2(e, 0.0)).y,
        shore_at(w, flat + float2(0.0, e)).y - shore_at(w, flat - float2(0.0, e)).y);
    return dot(g, g) > 1e-8 ? normalize(g) : float2(0.0);
}

// Train `t`'s table `distance` metres out from the shore: the seconds to the shore, the
// shoaling coefficient, and the local wave number (ω times the time's rate along the way).
float3 train_table(WaterSurface* w, uint t, float distance)
{
    uint n = w[0].shore_bins;
    float x = clamp(distance / w[0].shore_bin, 0.0, float(n - 1) - 0.001);
    uint j = uint(x);
    float f = x - float(j);
    float2 a = w[0].shore_table[t * n + j];
    float2 b = w[0].shore_table[t * n + j + 1];
    return float3(lerp(a, b, f), w[0].trains[t].x * (b.x - a.x) / w[0].shore_bin);
}

// What the trains do at a point.
struct ShoreWaves
{
    float height;   // metres over the mean level
    float2 offset;  // the horizontal displacement, world x and z, metres
    float2 slope;   // ∂height/∂x, ∂height/∂z
    float breaking; // 0 before the breaker line, rising as the trains break
    float foam;     // the broken water's cover, 0..1, before its pattern
    float swash;    // the sheet's height over the mean level up the beach, metres
    float2 inland;  // towards the shore (zero where the trains are not)
};

// Train `t`'s height before it breaks, `distance` metres out over `depth` metres of water: its
// height in deep water, shoaled, times the coast's `set`. Offshore the cascades carry the sea;
// the trains rise where the floor matters (under a fifth of their deep wavelength).
float train_height(WaterSurface* w, uint t, float distance, float depth, float set)
{
    float4 train = w[0].trains[t];
    float deep_wavelength = 2.0 * PI * GRAVITY / (train.x * train.x);
    float offshore = 1.0 - smoothstep(0.1 * deep_wavelength, 0.2 * deep_wavelength, depth);
    return train_table(w, t, distance).y * train.y * set * offshore;
}

// The trains at `flat` over `p`, for a caller that resolves `resolve` metres (the mesh's
// spacing, or the pixel's footprint): waves shorter than four of those fade to their mean.
ShoreWaves shore_waves(WaterSurface* w, float2 flat, ShorePoint p, float resolve)
{
    ShoreWaves s;
    s.height = 0.0;
    s.offset = 0.0;
    s.slope = 0.0;
    s.breaking = 0.0;
    s.foam = 0.0;
    s.swash = 0.0;
    s.inland = float2(0.0);
    uint count = min(w[0].train_count, WATER_MAX_TRAINS);
    if (count == 0 || w[0].shore == NO_SHORE || w[0].shore_table == nullptr || p.distance > SHORE_SWASH_REACH)
        return s;
    // Out where the floor is too deep for any train (see `offshore` below): nothing to do.
    float reach = 0.0;
    for (uint t = 0; t < count; t++)
        reach = max(reach, 0.2 * 2.0 * PI * GRAVITY / (w[0].trains[t].x * w[0].trains[t].x));
    if (p.depth >= reach)
        return s;
    float2 inland = shore_inland(w, flat);
    s.inland = inland;
    float2 world = flat + w[0].camera.xz;
    // Along the coast the sets rise and fall, and the crests drift a little out of line.
    float set = shore_set(world);
    float drift = shore_drift(world);
    float out_distance = max(-p.distance, 0.0);
    float depth = max(p.depth, 0.0);
    // First the trains' heights before breaking, and their run-up at the shoreline.
    float total2 = 0.0;
    float level = 0.0;
    for (uint t = 0; t < count; t++)
    {
        float4 train = w[0].trains[t];
        float height = train_height(w, t, out_distance, depth, set);
        total2 += height * height;
        level += SHORE_RUN_UP * train.y * set * shore_rush(train, w[0].time, drift);
    }
    // The breaker line: together the trains stand no higher than 0.78 of the depth.
    float total = sqrt(total2);
    float cap = BREAKER_INDEX * depth;
    float scale = total > cap ? cap / max(total, 1e-6) : 1.0;
    s.breaking = saturate(total / max(cap, 1e-3) - 1.0);
    // Then the waves. Gerstner's sharpening is shared out so the crests never loop
    // (Σ Q k a = 0.6: each train's share of the amplitudes, applied once they are summed).
    float sum = 0.0;
    float squeeze = 0.0;
    float along = 0.0;
    float push = 0.0;
    float foam = 0.0;
    for (uint t = 0; t < count; t++)
    {
        float4 train = w[0].trains[t];
        float3 table = train_table(w, t, out_distance);
        float number = max(table.z, 1e-4);
        float lod = saturate(2.0 * PI / number / (4.0 * resolve) - 1.0);
        float a = 0.5 * train_height(w, t, out_distance, depth, set) * scale * lod;
        if (a <= 0.0)
            continue;
        float phase = train.x * (w[0].time + table.x) + drift + train.z;
        float c = cos(phase);
        float sn = sin(phase);
        sum += a;
        s.height += a * c;
        push += (a / number) * sn;
        along += a * number * sn;
        squeeze += a * c;
        // White water on the front of each crest, left behind as it moves on.
        foam += a * exp(-5.0 * frac((phase + 0.6) / (2.0 * PI)));
    }
    if (sum > 0.0)
    {
        s.offset = inland * (0.6 / sum) * push;
        s.slope = inland * along / (1.0 - (0.6 / sum) * squeeze);
        s.foam = saturate(2.0 * s.breaking) * foam / sum;
    }
    // The sheet up the beach, and down over the last metres of sea, so the two meet.
    s.swash = level * shore_reach(p.distance);
    if (p.distance >= 0.0)
    {
        s.height = s.swash;
        s.offset = 0.0;
        s.slope = 0.0;
    }
    else
        s.height = max(s.height, s.swash * saturate(1.0 - out_distance / 6.0));
    return s;
}

// The broken water's pattern at `world`, in [0, 1], drifting shorewards: two octaves of value
// noise, each fading to its mean once it is finer than two of the pixel's `footprint`.
float foam_pattern(WaterSurface* w, float2 world, float2 inland, float footprint)
{
    float2 x = world - inland * (0.4 * w[0].time);
    float coarse = lerp(0.5, value_noise(x / 2.5), saturate(2.5 / (2.0 * footprint) - 1.0));
    float fine = lerp(0.5, value_noise(x / 0.9 + 31.7), saturate(0.9 / (2.0 * footprint) - 1.0));
    return smoothstep(0.25, 0.65, 0.65 * coarse + 0.35 * fine);
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
    ShorePoint shore = shore_point(w, flat);
    float3 d = displacement_at(w, flat, s * (1.0 + morph), shore.depth);
    ShoreWaves waves = shore_waves(w, flat, shore, s * (1.0 + morph));
    d += float3(waves.offset.x, waves.height, waves.offset.y);
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

// The light on a water surface of normal `n` and GGX roughness² `a2`, seen from `v`, besides
// what lies under it.
struct WaterLight
{
    float3 mirror;         // the mirror direction, kept above the horizon
    float fresnel;         // Schlick's, F0 0.02
    float blur;            // how far the reflection is blurred towards the sky's irradiance
    float3 reflected;      // the sky it reflects
    float3 sun;            // the sun's highlight
    float3 sun_light;      // the sun's light on the level surface
    float3 lit;            // the sky's and the sun's light on the level surface
    float scale;           // the sky's luminance scale
};

WaterLight water_light(WaterSurface* w, float3 n, float3 v, float a2)
{
    WaterLight o;
    float nv = max(dot(n, v), 1e-4);
    float f0 = 0.02;
    o.fresnel = f0 + (1.0 - f0) * pow(1.0 - nv, 5.0);
    float4 up_table = w[0].sky_light[SH_COEFFICIENTS];
    float3 up = up_table.xyz;
    float3 forward = w[0].sky_light[SH_COEFFICIENTS + 1].xyz;
    o.scale = w[0].sun_radiance.w;
    // The sky in the mirror direction (kept above the horizon: a wave's back cannot reflect
    // the sea), blurred towards the sky's irradiance as the surface roughens.
    float3 r = reflect(-v, n);
    r.y = abs(r.y);
    o.mirror = normalize(r);
    float3 mirror = sky_view_luminance(uint(up_table.w + 0.5), up, forward, o.mirror) * o.scale;
    float3 sky_irradiance = sh_irradiance(w[0].sky_light, float3(0.0, 1.0, 0.0)) * o.scale / PI;
    o.blur = saturate(2.0 * a2);
    o.reflected = lerp(mirror, sky_irradiance, o.blur);
    // The sun's highlight.
    float3 l = normalize(w[0].sun.xyz);
    float nl = dot(n, l);
    o.sun = 0.0;
    if (nl > 0.0)
    {
        float3 h = normalize(l + v);
        float fh = f0 + (1.0 - f0) * pow(1.0 - saturate(dot(v, h)), 5.0);
        o.sun = w[0].sun_radiance.rgb * ggx(saturate(dot(n, h)), a2) * smith_visibility(nv, nl, a2) * fh * nl;
    }
    o.sun_light = w[0].sun_radiance.rgb * saturate(l.y);
    o.lit = sky_irradiance * PI + o.sun_light;
    return o;
}

// What the surface writes (`trace_requested` in meshlet.rs traces the rays against the scene):
// - its colour;
// - the mirror ray it asks for: the direction, and the weight of what the ray meets in place
//   of the sky it reflected;
// - the shadow ray it asks for: the share of the colour the sun lights (rgb, which a blocked
//   ray takes away), and 1 in w, or zero where that share is too small to matter.
struct SurfaceTargets
{
    float4 color : SV_Target0;
    float4 request : SV_Target1;
    float4 sun : SV_Target2;
};

// Below this weight a mirror ray changes too little to trace.
static const float REQUEST_MIN_WEIGHT = 0.001;
// Below this share of the pixel's brightest channel, the sun's light is not worth a shadow ray.
static const float SUN_REQUEST_MIN_SHARE = 0.01;

[shader("fragment")]
SurfaceTargets surface_frag_main(SurfaceOut input)
{
    WaterSurface* w = surface_push.surface;
    // The slopes: their mean from every cascade's mips, and the variance of those the mips
    // average away (Bruneton, Neyret & Holzschuch 2010), each cascade damped by the depth as
    // its displacement is (the slopes scale with the amplitude, their variance with its square,
    // the Jacobian's departure from 1 to first order).
    ShorePoint shore = shore_point(w, input.flat);
    float2 slope = 0.0;
    float variance = 0.0;
    float jacobian = 1.0;
    for (uint c = 0; c < WATER_CASCADES; c++)
    {
        WaterCascadeView cv = w[0].cascades[c];
        float2 uv = input.flat * cv.inv_patch + cv.offset;
        float4 s = sample_image(cv.slopes, SAMPLER_ANISOTROPIC_REPEAT, uv);
        float a = shore_damping(cv, shore.depth);
        slope += a * s.xy;
        variance += a * a * max(s.w - dot(s.xy, s.xy), 0.0);
        jacobian += a * (s.z - 1.0);
    }
    // The shore's trains, resolved to the pixel.
    float footprint = max(max(length(ddx(input.flat)), length(ddy(input.flat))), 1e-3);
    ShoreWaves waves = shore_waves(w, input.flat, shore, footprint);
    slope += waves.slope;
    float3 n = normalize(float3(-slope.x, 1.0, -slope.y));
    float3 v = normalize(-input.relative);
    // GGX's α² from the unresolved slopes' variance (α² = 2σ²), over a calm surface's.
    float a2 = clamp(0.0004 + 2.0 * variance, 0.0004, 1.0);
    WaterLight light = water_light(w, n, v, a2);
    float fresnel = light.fresnel;
    float scale = light.scale;
    float3 r = light.mirror;
    float blur = light.blur;
    float3 reflected = light.reflected;
    float3 sun = light.sun;
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
    float3 sun_light = light.sun_light;
    float3 lit = light.lit;
    float3 inside = under * transmittance + w[0].scatter.rgb * lit * (1.0 - transmittance);
    float3 color = reflected * fresnel + inside * (1.0 - fresnel) + sun;
    // Foam, lit like a white diffuse surface: where the waves fold (the Jacobian under ~0.45),
    // where the shore's trains break, and along the swash's thin leading edge (the water's
    // depth under the pixel from the view ray's path to the floor), the shore's broken up by a
    // pattern that drifts shorewards.
    float foam = saturate((0.45 - jacobian) * 2.0);
    float thickness = path * max(v.y, 0.0);
    float edge = waves.swash > 0.02 ? 1.0 - smoothstep(0.0, 0.08, thickness) : 0.0;
    float white = saturate(1.5 * waves.foam + 0.7 * edge);
    if (white > 0.0)
        foam = max(foam, white * foam_pattern(w, input.flat + w[0].camera.xz, waves.inland, footprint));
    color = lerp(color, 0.8 * lit / PI, foam * 0.8);
    // The air between the camera and the water, as the compose hazes everything else.
    float4 air = aerial_at(w[0].sky, uv, surface_distance * 0.001);
    color = color * air.a + air.rgb * scale;
    SurfaceTargets o;
    o.color = float4(min(color, 32768.0), 1.0);    // The mirror ray: what it meets replaces `mirror`, whose share of the pixel is the Fresnel
    // term's, less the blur, the foam and the air.
    float weight = fresnel * (1.0 - blur) * (1.0 - foam * 0.8) * air.a;
    o.request = weight >= REQUEST_MIN_WEIGHT ? float4(r, weight) : float4(0.0);
    // The shadow ray: what the sun adds, through the same foam and air. The highlight, and the
    // sunlight the water scatters back (the light under a shaded surface loses the sun too);
    // the sea floor seen through the water was shaded by its own rays in the resolve.
    float3 sun_share = sun + w[0].scatter.rgb * sun_light * (1.0 - transmittance) * (1.0 - fresnel);
    sun_share = lerp(sun_share, 0.8 * sun_light / PI, foam * 0.8) * air.a;
    sun_share = min(sun_share, o.color.rgb);
    float brightest = max(o.color.r, max(o.color.g, o.color.b));
    bool shade = max(sun_share.r, max(sun_share.g, sun_share.b)) > SUN_REQUEST_MIN_SHARE * brightest;
    o.sun = shade ? float4(sun_share, 1.0) : float4(0.0);
    return o;
}

// ----------------------------------------------------------------- the rivers ---
//
// D-038's rivers: each river of the island's network as a ribbon over the ground
// (`forge_procgen::ribbons`: the D8 course smoothed, its width, depth and speed from the
// catchment and the bed's slope), drawn after the sea in the same pass and blended over what is
// there by how much of the pixel it covers. The 8 m field cannot hold a bed a few metres wide,
// so the water lies on the ground as the island's mesh draws it, a little above it, and its
// depth is the ribbon's own profile across rather than the ground's: the outline is the
// ribbon's, smooth, and not the triangles'. Its ripples are the finest cascade's slopes carried
// down the river by a flow map (Vlachos 2010): two samples advected along the flow, each
// restarted in turn half a period apart and cross-faded, so the texture never stretches. Far
// away the ribbon stays a pixel wide either side, its coverage scaled by the share of that the
// river fills (Persson's phone-wire anti-aliasing), so a stream thinner than a pixel stays a
// steady faint line; it rises by the pixel's footprint, over what the ground's coarser levels
// of detail may raise.

// Quads across a ribbon (`RIVER_ACROSS` in water.rs).
static const uint RIVER_ACROSS = 4;
// Metres over the ground it rests on: a larger river is lifted more, so it covers the end of a
// tributary.
static const float RIVER_LIFT = 0.1;
static const float RIVER_LIFT_PER_DEPTH = 0.25;
// Far away: the lift in pixels' footprints.
static const float RIVER_LIFT_PIXELS = 1.5;
// Seconds between the restarts of a flow sample.
static const float RIVER_FLOW_PERIOD = 2.0;
// The ripples' tile against the finest cascade's patch, and their slopes' scale.
static const float RIVER_RIPPLE_TILE = 0.5;
static const float RIVER_RIPPLE_STRENGTH = 0.35;
// Fresh water with a little tannin and silt (m⁻¹), and the light it scatters back.
static const float3 RIVER_ABSORPTION = float3(1.4, 0.4, 0.5);
static const float3 RIVER_SCATTER = float3(0.015, 0.025, 0.02);
// The bed under the water towards the middle: the ground's light on wet, grey-brown sediment.
static const float3 RIVER_BED = float3(0.55, 0.5, 0.42);
// Metres of the banks' soft edge, at least (and two pixels far away).
static const float RIVER_BANK = 0.5;
// Metres between the riffles' white water, and the foam's cover where its pattern is too fine
// to draw (the pattern's mean through the threshold).
static const float RIVER_RIFFLES = 30.0;
static const float RIVER_FOAM_MEAN = 0.3;
// The river gives way to the sea over its last metres to the coast.
static const float RIVER_MOUTH = 30.0;

struct RiverOut
{
    float4 position : SV_Position;
    float3 relative : POSITION;   // camera-relative metres
    float2 flat : TEXCOORD0;      // camera-relative world x, z
    float4 river : TEXCOORD1;     // across (−1..1), the coverage (the thinning far away, the fade), the depth (m), the speed (m/s)
    float4 flow : TEXCOORD2;      // downstream (world x, z), the bed's slope, the half width drawn (m)
};

// The ground's height at `world` (x, z) as the island's mesh draws it: the field's samples, each
// cell split along its (i + 1, j) – (i, j + 1) diagonal (`forge_geom::city::heightfield_mesh`).
float ground_height(WaterSurface* w, float2 world)
{
    uint n = w[0].shore_texels;
    float2 g = (world - w[0].shore_frame.xy) * w[0].shore_frame.z;
    float2 cell = clamp(floor(g), 0.0, float(n - 2u));
    float2 t = saturate(g - cell);
    uint i = uint(cell.y) * n + uint(cell.x);
    float a = w[0].ground[i];
    float b = w[0].ground[i + 1u];
    float c = w[0].ground[i + n];
    float d = w[0].ground[i + n + 1u];
    return t.x + t.y <= 1.0
        ? a + t.x * (b - a) + t.y * (c - a)
        : d + (1.0 - t.x) * (c - d) + (1.0 - t.y) * (b - d);
}

[shader("vertex")]
RiverOut river_vert_main(uint id : SV_VertexID)
{
    WaterSurface* w = surface_push.surface;
    uint segment = id / (RIVER_ACROSS * 6u);
    uint rest = id % (RIVER_ACROSS * 6u);
    // x across the ribbon, y along it.
    int2 corner = QUAD_CORNERS[rest % 6u] + int2(int(rest / 6u), 0);
    RiverOut o;
    if (w[0].rivers[segment].c.y == 0.0)
    {
        o.position = float4(0.0, 0.0, 0.0, 0.0); // a river's last point: no segment
        o.relative = 0.0;
        o.flat = 0.0;
        o.river = 0.0;
        o.flow = 0.0;
        return o;
    }
    RiverPoint p = w[0].rivers[segment + uint(corner.y)];
    float3 camera = w[0].camera.xyz;
    float2 centre = p.a.xy;
    float footprint = length(float3(centre.x, ground_height(w, centre), centre.y) - camera) * w[0].pixel;
    // At least a pixel either side, the coverage scaled to what the river fills.
    float half_width = max(p.a.z, footprint);
    float across = float(corner.x) / float(RIVER_ACROSS) * 2.0 - 1.0;
    float2 world = centre + float2(-p.b.y, p.b.x) * (across * half_width);
    // On the ground under the ribbon's quads (and, widened, under the vertex where it went).
    float resting = corner.x < 4 ? p.d[corner.x] : p.c.z;
    float height = max(resting, ground_height(w, world)) + RIVER_LIFT + RIVER_LIFT_PER_DEPTH * p.a.w + RIVER_LIFT_PIXELS * footprint;
    float3 relative = float3(world.x, height, world.y) - camera;
    o.position = mul(w[0].view_proj, float4(relative, 1.0));
    o.relative = relative;
    o.flat = relative.xz;
    o.river = float4(across, p.c.x * p.a.z / half_width, p.a.w, p.b.z);
    o.flow = float4(p.b.xy, p.b.w, half_width);
    return o;
}

// Two samples of a texture advected along `velocity` (Vlachos 2010): the phases of each, a
// period apart, and their weights, each sample's zero where it restarts.
struct FlowPhases
{
    float2 offset0; // metres each sample has drifted
    float2 offset1;
    float weight0;
    float weight1;
};

FlowPhases flow_phases(float2 world, float2 velocity, float time)
{
    // Each place restarts at its own time, so the cross-fade does not pulse over the river.
    float phase = time / RIVER_FLOW_PERIOD + value_noise(world / 5.0);
    float p0 = frac(phase);
    float p1 = frac(phase + 0.5);
    FlowPhases f;
    f.offset0 = velocity * (p0 * RIVER_FLOW_PERIOD);
    f.offset1 = velocity * (p1 * RIVER_FLOW_PERIOD);
    f.weight0 = 1.0 - abs(1.0 - 2.0 * p0);
    f.weight1 = 1.0 - f.weight0;
    return f;
}

[shader("fragment")]
SurfaceTargets river_frag_main(RiverOut input)
{
    WaterSurface* w = surface_push.surface;
    float across = input.river.x;
    float2 world = input.flat + w[0].camera.xz;
    // How much of the pixel the river covers: its banks, soft over half a metre or two pixels
    // but never more than half the half width drawn (far away the whole ribbon is two pixels,
    // and its coverage must add up to the river's share), its thinning far away and its fade
    // from the head; over its last metres to the coast it gives way to the sea.
    ShorePoint shore = shore_point(w, input.flat);
    float footprint = max(max(length(ddx(input.flat)), length(ddy(input.flat))), 1e-3);
    float bank = (1.0 - abs(across)) * input.flow.w;
    float soft = min(max(RIVER_BANK, 2.0 * footprint), 0.5 * input.flow.w);
    float coverage = input.river.y * smoothstep(0.0, soft, bank)
        * smoothstep(0.0, RIVER_MOUTH, shore.distance);
    // The flow: faster in the middle than by the banks.
    float2 down = input.flow.xy / max(length(input.flow.xy), 1e-4);
    float2 velocity = down * (input.river.w * (1.0 - 0.4 * across * across));
    FlowPhases f = flow_phases(world, velocity, w[0].time);
    // The ripples: the finest cascade's slopes on a smaller tile, carried downstream. Cross-faded,
    // with the contrast two unrelated samples lose halfway put back.
    WaterCascadeView cv = w[0].cascades[WATER_CASCADES - 1];
    float tile = cv.inv_patch / RIVER_RIPPLE_TILE;
    float2 offset = cv.offset / RIVER_RIPPLE_TILE;
    float4 s0 = sample_image(cv.slopes, SAMPLER_ANISOTROPIC_REPEAT, (input.flat - f.offset0) * tile + offset);
    float4 s1 = sample_image(cv.slopes, SAMPLER_ANISOTROPIC_REPEAT, (input.flat - f.offset1) * tile + offset + float2(0.37, 0.61));
    float norm = rsqrt(f.weight0 * f.weight0 + f.weight1 * f.weight1);
    float strength = RIVER_RIPPLE_STRENGTH / RIVER_RIPPLE_TILE;
    float2 ripple = (s0.xy * f.weight0 + s1.xy * f.weight1) * (norm * strength);
    float variance = (f.weight0 * max(s0.w - dot(s0.xy, s0.xy), 0.0) + f.weight1 * max(s1.w - dot(s1.xy, s1.xy), 0.0))
        * (strength * strength);
    // The surface falls downstream with its bed.
    float2 slope = ripple - down * min(input.flow.z, 0.3);
    float3 n = normalize(float3(-slope.x, 1.0, -slope.y));
    float3 v = normalize(-input.relative);
    float a2 = clamp(0.0004 + 2.0 * variance, 0.0004, 1.0);
    WaterLight light = water_light(w, n, v, a2);
    // What lies under: the ground, its light on a bed of sediment towards the middle, seen
    // through the ribbon's depth (a parabola across, deepest in the middle) along the view ray.
    uint2 pixel = uint2(input.position.xy);
    float2 uv = (float2(pixel) + 0.5) / float2(w[0].width, w[0].height);
    float3 under = g_sampled_images[NonUniformResourceIndex(w[0].scene_color)].Load(int3(pixel, 0)).rgb;
    float luminance = dot(under, float3(0.2126, 0.7152, 0.0722));
    under = lerp(under, luminance * RIVER_BED, 1.0 - smoothstep(0.3, 0.95, abs(across)));
    float depth = input.river.z * (1.0 - across * across);
    float3 transmittance = exp(-RIVER_ABSORPTION * (depth / max(v.y, 0.2)));
    float3 inside = under * transmittance + RIVER_SCATTER * light.lit * (1.0 - transmittance);
    float3 color = light.reflected * light.fresnel + inside * (1.0 - light.fresnel) + light.sun;
    // White water in riffles where the bed is steep and the stream fast, broken up by a pattern
    // carried down with the flow; finer than two pixels, the pattern gives way to its mean.
    float riffles = smoothstep(0.45, 0.75, value_noise(world / RIVER_RIFFLES));
    float rapids = smoothstep(0.06, 0.2, input.flow.z) * smoothstep(1.5, 3.0, input.river.w) * riffles;
    float foam = 0.0;
    if (rapids > 0.0)
    {
        float n0 = value_noise((world - f.offset0) / 0.8);
        float n1 = value_noise((world - f.offset1) / 0.8 + 13.7);
        float pattern = 0.5 + ((n0 - 0.5) * f.weight0 + (n1 - 0.5) * f.weight1) * norm;
        float resolved = saturate(0.8 / (2.0 * footprint) - 1.0);
        foam = rapids * lerp(RIVER_FOAM_MEAN, smoothstep(0.45, 0.8, pattern), resolved);
    }
    color = lerp(color, 0.8 * light.lit / PI, foam * 0.8);
    float4 air = aerial_at(w[0].sky, uv, length(input.relative) * 0.001);
    color = color * air.a + air.rgb * light.scale;
    // Blended by the coverage, as are the rays asked for (`trace_requested` normalises the
    // direction): what the mirror ray meets and what the sun lights of the river's share.
    SurfaceTargets o;
    o.color = float4(min(color, 32768.0), coverage);
    float weight = light.fresnel * (1.0 - light.blur) * (1.0 - foam * 0.8) * air.a * coverage;
    o.request = weight >= REQUEST_MIN_WEIGHT ? float4(light.mirror, weight) : float4(0.0);
    float3 sun_share = light.sun + RIVER_SCATTER * light.sun_light * (1.0 - transmittance) * (1.0 - light.fresnel);
    sun_share = lerp(sun_share, 0.8 * light.sun_light / PI, foam * 0.8) * air.a;
    sun_share = min(sun_share, o.color.rgb) * coverage;
    float brightest = max(o.color.r, max(o.color.g, o.color.b)) * coverage;
    bool shade = max(sun_share.r, max(sun_share.g, sun_share.b)) > SUN_REQUEST_MIN_SHARE * brightest;
    o.sun = shade ? float4(sun_share, 1.0) : float4(0.0);
    // Nothing of the river here: leave the pixel, its depth and its rays to what is under.
    if (coverage < 1.0 / 255.0)
        discard;
    return o;
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
