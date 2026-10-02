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
    float4 a; // world x, z (the sea's frame), the half width at the water's edge (m), the depth in the middle (m)
    float4 b; // downstream (world x, z; unit), the speed (m/s), the water surface's slope
    float4 c; // how much of the river is drawn (0..1), the half width drawn whole where a segment starts (m; 0 at a river's last point: none starts), the ground under the last vertex across, the water's level (m)
    float4 d; // the ground under the first four vertices across: the highest the drawn ground stands under the quads around each (m)
    float4 e; // the ribbon's half width (m, under the banks), the lowest bank before the channel was carved (m), the first stone past the point, the metres along the river from its head
    float4 f; // the white water a step's fall leaves (0..1, #122), how far its line bows downstream in the middle and towards the left bank (m; negative: the right), one unused
};

// Where a river's water meets the sea's (`GpuRiverMouth` in water.rs).
struct RiverMouth
{
    float4 a; // world x, z (the sea's frame), downstream (world x, z; unit)
    float4 b; // the half width (m), the speed (m/s), the white water's share (0..1), 0
};

// A lake (`GpuLake` in water.rs).
struct Lake
{
    float4 a; // the mask's first sample (world x, z), the level (m), metres between samples
    uint4 b;  // the mask's samples along x and z, its first word in `lake_masks`, its depth (m, as float bits)
};

// A stone in a river (`GpuRiverStone` in water.rs).
struct RiverStone
{
    float4 a; // world x, z (the sea's frame), the radius of its outline at the water's level (m; 0 under it), its mean radius (m)
};

// Something floating in the water this frame (`GpuFloater` in water.rs, #107).
struct Floater
{
    float4 a; // world x, z (the sea's frame), the radius of its outline at the water's level (m), 0
    float4 b; // its velocity (world x, z; m/s), 0, 0
};

// The water at the camera (`water/at-camera`, #108): its surface there as a plane, and its
// water's colour.
struct WaterAtCamera
{
    float4 plane;      // the surface's height over the camera (m), its slopes ∂h/∂x and ∂h/∂z, 0
    float4 absorption; // rgb: its water's absorption, m⁻¹
    float4 scatter;    // rgb: the light its water scatters back, per unit of irradiance
    uint4 under_groups; // `water/under`'s dispatch: its groups along x and y (none while the near plane stands over the water), 1, 0
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
    RiverMouth* mouths;   // where the rivers meet the sea, `mouth_count` of them
    uint mouth_count;
    uint stone_count;
    RiverStone* stones;   // the stones in the rivers, in the order of the points they stand past
    uint* mouth_grid;     // per cell of the shore's square, up to four mouths whose plume reaches it (a byte each, 0xFF: none)
    uint mouth_cells;     // the grid's cells a side
    float mouth_cell;     // metres a cell
    Lake* lakes;          // the lakes, `lake_count` of them
    uint* lake_masks;     // their masks, a bit a sample
    uint lake_count;
    uint pad_lakes;
    WaterAtCamera* at_camera; // the water at the camera, or null: the camera stands too high to be under it
    uint2 pad_at_camera;
    float4 fresh;         // the fresh water the camera stands in or over: its level (m), 1 a lake or 2 a river (0: the sea's), 0, 0
    Floater* floaters;    // what floats in the water this frame (#107)
    uint* floater_cells;  // their grid around the camera: per cell its list's first entry and count (16 bits each), then the lists (indices into `floaters`)
    float4 floater_grid;  // the grid's origin (world x, z), 1 / metres a cell, cells a side (0: no floaters)
    float4 wake_window;   // the wakes' field (#107, `wakes.slang`): its corner (world x, z), 1 / its size in metres (0: no wakes), 0
    uint wake_slopes;     // sampled index (rg16f): the field's slopes
    uint3 pad_wake;
};

struct SurfacePush
{
    WaterSurface* surface;
    uint first;  // the sea: the clipmap level drawn; the rivers: the first vertex of the run of segments drawn
    uint pad;
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

// How much of cascade `v`'s waves stand in `depth` metres of water: 1 at the spectrum's own
// depth and deeper, 0 on land (`tma` in shore.slang).
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

// A vertex per point of a level's lattice, `(grid + 1)²` of them, the level in the push block's
// `first`: the draws index them by the quads in view, less those the finer level covers
// (`surface_indices` in water.rs).
[shader("vertex")]
SurfaceOut surface_vert_main(uint id : SV_VertexID)
{
    WaterSurface* w = surface_push.surface;
    uint g = w[0].grid;
    uint side = g + 1u;
    uint level = surface_push.first;
    int half_grid = int(g / 2u);
    int2 lattice = int2(int(id % side), int(id / side)) - half_grid; // from the level's centre, in spacings
    float4 l = w[0].levels[level];
    float s = l.z;
    float2 centre = l.xy;
    SurfaceOut o;
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

// Water's index of refraction.
static const float WATER_IOR = 1.333;

// How steeply the view ray falls once it crosses a water surface of normal `n` seen from `v`
// (Snell's law): the refracted ray's downward share, at least 0.1. Straight down it is 1; at a
// grazing angle about 0.66, where the straight ray's is near 0.
float refracted_down(float3 n, float3 v)
{
    float3 t = refract(-v, n, 1.0 / WATER_IOR);
    return max(-t.y, 0.1);
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
// The swash's leading edge: the sheet's thickness (m) under which its foam line shows, and how
// much the foam's pattern breaks the line up (a share of its cover).
static const float SWASH_EDGE = 0.03;
static const float SWASH_EDGE_BREAKUP = 0.4;

// The cascades' slopes at a point of the sea.
struct SeaSlopes
{
    float2 slope;    // ∂h/∂x, ∂h/∂z
    float variance;  // the slopes' the mips average away
    float jacobian;  // under ~0.45, the waves fold: foam
};

// The wakes' slopes at `world` (#107, `wakes.slang`): what moves through the lakes and the sea
// makes waves half a metre long, fading out over the field's last tenth, and where a pixel
// spans more than an eighth of their length, gone at half of it, their Nyquist limit: past
// it the crests would only shimmer (the field holds no mips).
float2 wake_slope(WaterSurface* w, float2 world, float footprint)
{
    float4 window = w[0].wake_window;
    if (window.z <= 0.0)
        return 0.0;
    float2 uv = (world - window.xy) * window.z;
    float edge = min(min(uv.x, uv.y), 1.0 - max(uv.x, uv.y));
    float fade = smoothstep(0.0, 0.1, edge) * (1.0 - smoothstep(0.06, 0.25, footprint));
    if (fade <= 0.0)
        return 0.0;
    return sample_image_level(w[0].wake_slopes, SAMPLER_LINEAR_CLAMP, uv, 0.0).xy * fade;
}

// The slopes: their mean from every cascade's mips, and the variance of those the mips average
// away (Bruneton, Neyret & Holzschuch 2010), each cascade damped by the depth as its
// displacement is (the slopes scale with the amplitude, their variance with its square, the
// Jacobian's departure from 1 to first order).
SeaSlopes sea_slopes(WaterSurface* w, float2 flat, ShorePoint shore)
{
    SeaSlopes o;
    o.slope = 0.0;
    o.variance = 0.0;
    o.jacobian = 1.0;
    for (uint c = 0; c < WATER_CASCADES; c++)
    {
        WaterCascadeView cv = w[0].cascades[c];
        float2 uv = flat * cv.inv_patch + cv.offset;
        float4 s = sample_image(cv.slopes, SAMPLER_ANISOTROPIC_REPEAT, uv);
        float a = shore_damping(cv, shore.depth);
        o.slope += a * s.xy;
        o.variance += a * a * max(s.w - dot(s.xy, s.xy), 0.0);
        o.jacobian += a * (s.z - 1.0);
    }
    return o;
}

[shader("fragment")]
SurfaceTargets surface_frag_main(SurfaceOut input)
{
    return sea_from_above(surface_push.surface, input);
}

// The sea when the camera stands low enough to be under it (#108, "under the water" below):
// every pixel is shaded as seen from above first, so the derivatives that shading takes see
// the whole quad, then from below where the pixel looks out from under the water.
[shader("fragment")]
SurfaceTargets surface_under_frag_main(SurfaceOut input)
{
    WaterSurface* w = surface_push.surface;
    SurfaceTargets o = sea_from_above(w, input);
    WaterAtCamera at = w[0].at_camera[0];
    if (under_by(at, near_point(w, input.position.xy)) > 0.0)
        o = sea_from_below(w, input, at);
    return o;
}

SurfaceTargets sea_from_above(WaterSurface* w, SurfaceOut input)
{
    ShorePoint shore = shore_point(w, input.flat);
    SeaSlopes cascades = sea_slopes(w, input.flat, shore);
    float2 slope = cascades.slope;
    float variance = cascades.variance;
    float jacobian = cascades.jacobian;
    // The shore's trains, resolved to the pixel.
    float footprint = max(max(length(ddx(input.flat)), length(ddy(input.flat))), 1e-3);
    ShoreWaves waves = shore_waves(w, input.flat, shore, footprint);
    slope += waves.slope;
    // What moves through it (#107).
    slope += wake_slope(w, input.flat + w[0].camera.xz, footprint);
    // Where a river comes in: its water, and its ripples carried on its flow, as the river's
    // ribbon draws them where it hands over to the sea (in its channel, where the sea's own waves
    // are still), mixing out into the sea's in a plume.
    Estuary estuary = estuary_at(w, input.flat + w[0].camera.xz);
    if (estuary.fresh > 0.0)
    {
        RiverRipples ripples = river_ripples(w, input.flat, estuary.velocity);
        slope += ripples.slope * estuary.fresh;
        variance += ripples.variance * estuary.fresh;
    }
    // The river's water lies over the sea's in a layer that thins as it mixes out: all of the
    // water in the channel, a metre and a half at the mouth, less out to sea.
    float layer = estuary.fresh * RIVER_PLUME_LAYER;
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
    // What lies under the water: the scene behind this pixel, dimmed along the path the light
    // takes through the water to it, and the light the water scatters back on the way. The view
    // ray meets the floor `thickness` under the surface; the light bends down into the water
    // (`refracted_down`), so its path there is that depth over the refracted ray's fall, not
    // the straight ray's: a little longer than the depth, where the straight ray at a grazing
    // angle went ten times as far and every wave changed it by metres.
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
    float thickness = path * max(v.y, 0.0);
    float fall = refracted_down(n, v);
    path = thickness / fall;
    float layer_path = min(path, layer / fall);
    float3 layer_through = exp(-RIVER_ABSORPTION * layer_path);
    float3 sea_through = exp(-w[0].absorption.rgb * (path - layer_path));
    float3 under = g_sampled_images[NonUniformResourceIndex(w[0].scene_color)].Load(int3(pixel, 0)).rgb;
    float3 sun_light = light.sun_light;
    float3 lit = light.lit;
    float3 inside = (under * sea_through + w[0].scatter.rgb * lit * (1.0 - sea_through)) * layer_through
        + RIVER_SCATTER * lit * (1.0 - layer_through);
    float3 color = reflected * fresnel + inside * (1.0 - fresnel) + sun;
    // Foam, lit like a white diffuse surface: where the waves fold (the Jacobian under ~0.45),
    // where the shore's trains break, broken up by a pattern that drifts shorewards, and along
    // the swash's thin leading edge (the water's depth under the pixel from the view ray's path
    // to the floor), a line the pattern only frays: cut into the pattern's blobs, a band that
    // narrow reads as a dashed line.
    float foam = saturate((0.45 - jacobian) * 2.0);
    float edge = waves.swash > 0.02 ? 1.0 - smoothstep(0.0, SWASH_EDGE, thickness) : 0.0;
    float breaking = saturate(1.5 * waves.foam);
    if (breaking > 0.0 || edge > 0.0)
    {
        float pattern = foam_pattern(w, input.flat + w[0].camera.xz, waves.inland, footprint);
        foam = max(foam, breaking * pattern);
        foam = max(foam, 0.7 * edge * (1.0 - SWASH_EDGE_BREAKUP * (1.0 - pattern)));
    }
    // A river's white water, spilling out of its mouth: the pattern drifts out with its flow.
    if (estuary.white > 0.0)
    {
        float speed = length(estuary.velocity);
        float2 out_to_sea = speed > 1e-4 ? estuary.velocity / speed : float2(0.0);
        foam = max(foam, estuary.white * foam_pattern(w, input.flat + w[0].camera.xz, out_to_sea, footprint));
    }
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
    float3 sun_share = sun + (w[0].scatter.rgb * sun_light * (1.0 - sea_through) * layer_through
        + RIVER_SCATTER * sun_light * (1.0 - layer_through)) * (1.0 - fresnel);
    sun_share = lerp(sun_share, 0.8 * sun_light / PI, foam * 0.8) * air.a;
    sun_share = min(sun_share, o.color.rgb);
    float brightest = max(o.color.r, max(o.color.g, o.color.b));
    bool shade = max(sun_share.r, max(sun_share.g, sun_share.b)) > SUN_REQUEST_MIN_SHARE * brightest;
    o.sun = shade ? float4(sun_share, 1.0) : float4(0.0);
    return o;
}

// ----------------------------------------------------------------- the rivers ---
//
// D-038's rivers: each river of the island's network as a ribbon (`forge_procgen::ribbons`:
// the D8 course smoothed, its width, depth and speed from the catchment and its water's fall),
// drawn after the sea in the same pass and blended over what is there by how much of the pixel
// it covers. The water is level across, at a level that only falls downstream and stands under
// its banks, in the channel the island's mesh carves for it (`forge_procgen::Channels`): the
// ribbon reaches past the water's edge under the banks, so the ground draws the edge, and what
// lies under it is the bed, seen through the water's depth as the sea sees its floor. Its
// ripples are the finest cascade's slopes carried down the river by a flow map (Vlachos 2010):
// two samples advected along the flow, each restarted in turn half a period apart and
// cross-faded, so the texture never stretches; still water is the same ripples unmoved. Far
// away, where the ground's coarser levels of detail may fill the channel, the ribbon rises onto
// the ground as drawn and stays a pixel wide either side, its coverage scaled by the share of
// that the river fills (Persson's phone-wire anti-aliasing), so a stream thinner than a pixel
// stays a steady faint line.

// Quads across a ribbon (`RIVER_ACROSS` in water.rs).
static const uint RIVER_ACROSS = 4;
// Far away: the lift over the ground in pixels' footprints.
static const float RIVER_LIFT_PIXELS = 1.5;
// The footprint, as a share of the channel's depth under its banks, over which the ribbon
// rises from its level onto the ground.
static const float2 RIVER_DRAPE = float2(0.2, 0.8);
// Seconds between the restarts of a flow sample.
static const float RIVER_FLOW_PERIOD = 2.0;
// The ripples' tile against the finest cascade's patch, and their slopes' scale.
static const float RIVER_RIPPLE_TILE = 0.5;
static const float RIVER_RIPPLE_STRENGTH = 0.35;
// Fresh water with a little tannin and silt (m⁻¹): the bed shows through a metre of it.
static const float3 RIVER_ABSORPTION = float3(0.9, 0.35, 0.45);
// A river's bed of grey-brown gravel: the ground under the water takes this tint at its own
// brightness, for this share (#114).
static const float4 RIVER_BED = float4(1.12, 1.0, 0.8, 0.85);
static const float3 RIVER_SCATTER = float3(0.02, 0.026, 0.018);
// Metres of the banks' soft edge far away, at least (and two pixels).
static const float RIVER_BANK = 0.5;
// The water's depth over which its edge fades where it meets the ground or anything in it:
// metres, and pixels' footprints.
static const float2 RIVER_CONTACT = float2(0.02, 0.25);
// Metres more than its own depth that a river or a lake can stand over what the view ray meets
// under it: past that the ground is not drawn there yet.
static const float RIVER_DEEPEST = 5.0;
// How much rougher a stone's wake is (added to the slopes' variance).
static const float RIVER_WAKE_ROUGHNESS = 0.02;
// Metres between the riffles' white water, and the foam's cover where its pattern is too fine
// to draw (the pattern's mean through the threshold).
static const float RIVER_RIFFLES = 30.0;
static const float RIVER_FOAM_MEAN = 0.3;

struct RiverOut
{
    float4 position : SV_Position;
    float3 relative : POSITION;   // camera-relative metres
    float2 flat : TEXCOORD0;      // camera-relative world x, z
    float4 river : TEXCOORD1;     // metres from the middle, positive to the left (as metres: a share of the half width times it, interpolated per triangle, drew teeth where the width changes fast, #119), the fade, the depth in the middle (m), the speed (m/s)
    float4 flow : TEXCOORD2;      // downstream (world x, z), the water surface's slope, the half width drawn (m)
    float4 edge : TEXCOORD3;      // the half width at the water's edge (m), the ribbon's (m), how far it has risen onto the ground (0..1), the metres along the river
    nointerpolation uint segment : TEXCOORD4; // the segment's first point
    float cover : TEXCOORD5;      // the half width drawn whole (m): the water's edge's, more in a confluence's rounded corner (#119)
    float foam : TEXCOORD6;       // the white water a step's fall leaves (0..1, #122)
};

// The ground's height at `world` (x, z) as the island's mesh draws its coarse cells: the field's
// samples, each cell split along its (i + 1, j) – (i, j + 1) diagonal
// (`forge_geom::city::heightfield_mesh`).
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
RiverOut river_vert_main(uint vertex : SV_VertexID)
{
    WaterSurface* w = surface_push.surface;
    uint id = vertex + surface_push.first;
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
        o.edge = 0.0;
        o.segment = 0u;
        o.cover = 0.0;
        o.foam = 0.0;
        return o;
    }
    RiverPoint p = w[0].rivers[segment + uint(corner.y)];
    float3 camera = w[0].camera.xyz;
    float2 centre = p.a.xy;
    float level = p.c.w;
    float footprint = length(float3(centre.x, level, centre.y) - camera) * w[0].pixel;
    // Onto the ground as the pixel grows past a share of the channel's depth under its banks.
    float channel = max(p.e.y - (level - p.a.w), p.a.w);
    float drape = smoothstep(RIVER_DRAPE.x, RIVER_DRAPE.y, footprint / channel);
    // At least a pixel either side, the coverage scaled to what the river fills.
    float half_width = max(p.e.x, footprint);
    float across = float(corner.x) / float(RIVER_ACROSS) * 2.0 - 1.0;
    float2 world = centre + float2(-p.b.y, p.b.x) * (across * half_width);
    // A step's line bowed downstream (#122): an arch and a slant towards a bank, at the vertices
    // across (`forge_procgen::lip_shift`, which the carve under it follows).
    float slant = p.f.z >= 0.0 ? 1.0 + across : 1.0 - across;
    world += p.b.xy * (p.f.y * (1.0 - across * across) + abs(p.f.z) * 0.5 * slant);
    // Far away, on the ground under the ribbon's quads (and, widened, under the vertex).
    float resting = corner.x < 4 ? p.d[corner.x] : p.c.z;
    float draped = max(resting, ground_height(w, world)) + RIVER_LIFT_PIXELS * footprint;
    float height = lerp(level, max(level, draped), drape);
    float3 relative = float3(world.x, height, world.y) - camera;
    o.position = mul(w[0].view_proj, float4(relative, 1.0));
    o.relative = relative;
    o.flat = relative.xz;
    o.river = float4(across * half_width, p.c.x, p.a.w, p.b.z);
    o.flow = float4(p.b.xy, p.b.w, half_width);
    o.edge = float4(p.a.z, p.e.x, drape, p.e.w);
    o.segment = segment;
    // At a river's last point (its `c.y` 0: no segment starts there), its half width.
    o.cover = max(p.c.y, p.a.z);
    o.foam = p.f.x;
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

// The ripples of fresh water moving at `velocity` (world x, z; m/s) at `flat` (camera-relative
// world x, z): the finest cascade's slopes on a smaller tile, carried downstream, cross-faded
// with the contrast two unrelated samples lose halfway put back. Unmoved, the same ripples
// stand still: the river's, a lake's and the sea's at a river's mouth are one surface.
struct RiverRipples
{
    float2 slope;
    float variance;   // of the slopes the mips average away
};

RiverRipples river_ripples(WaterSurface* w, float2 flat, float2 velocity)
{
    RiverRipples o;
    FlowPhases f = flow_phases(flat + w[0].camera.xz, velocity, w[0].time);
    WaterCascadeView cv = w[0].cascades[WATER_CASCADES - 1];
    float tile = cv.inv_patch / RIVER_RIPPLE_TILE;
    float2 offset = cv.offset / RIVER_RIPPLE_TILE;
    float4 s0 = sample_image(cv.slopes, SAMPLER_ANISOTROPIC_REPEAT, (flat - f.offset0) * tile + offset);
    float4 s1 = sample_image(cv.slopes, SAMPLER_ANISOTROPIC_REPEAT, (flat - f.offset1) * tile + offset + float2(0.37, 0.61));
    float norm = rsqrt(f.weight0 * f.weight0 + f.weight1 * f.weight1);
    float strength = RIVER_RIPPLE_STRENGTH / RIVER_RIPPLE_TILE;
    o.slope = (s0.xy * f.weight0 + s1.xy * f.weight1) * (norm * strength);
    o.variance = (f.weight0 * max(s0.w - dot(s0.xy, s0.xy), 0.0) + f.weight1 * max(s1.w - dot(s1.xy, s1.xy), 0.0))
        * (strength * strength);
    return o;
}

// A river's water in the sea (D-038's rivers, where they meet the sea): how much of the water at
// a point is the river's (0..1) and the flow it still carries.
struct Estuary
{
    float fresh;
    float2 velocity; // world x, z; m/s
    float white;     // the white water a river brings, before the sea's foam pattern
};

// How far a river's plume reaches, in its half widths, and how fast it spreads (metres of half
// width a metre out: a turbulent jet's).
static const float RIVER_PLUME_LENGTH = 60.0;
static const float RIVER_PLUME_SPREAD = 0.2;
// How far the plume's axis wanders, in metres a metre out at most, and the metres of river water
// lying over the sea's where the plume leaves the mouth.
static const float RIVER_PLUME_WANDER = 0.25;
static const float RIVER_PLUME_LAYER = 1.5;
// Half widths out to sea over which the white water a river brings fades.
static const float RIVER_SPILL = 6.0;

// The rivers' water at `world` (x, z). Up a river's channel from where its level reaches the
// sea's, where the sea fills it, the water is the river's and moves at its speed. Out from there it is a jet that widens and slows (a plane
// jet's centre speed falls as the root of the distance), its water mixing into the sea's, the
// plume's edge broken up offshore. Only the mouths the grid lists for the point's cell are
// looked at.
Estuary estuary_at(WaterSurface* w, float2 world)
{
    Estuary e;
    e.fresh = 0.0;
    e.velocity = 0.0;
    e.white = 0.0;
    if (w[0].mouth_count == 0u)
        return e;
    int2 cell = int2(floor((world - w[0].shore_frame.xy) / w[0].mouth_cell));
    if (any(cell < 0) || any(cell >= int(w[0].mouth_cells)))
        return e;
    uint listed = w[0].mouth_grid[uint(cell.y) * w[0].mouth_cells + uint(cell.x)];
    for (uint k = 0; k < 4u; k++)
    {
        uint m = (listed >> (8u * k)) & 0xFFu;
        if (m == 0xFFu)
            break;
        RiverMouth mouth = w[0].mouths[m];
        float2 d = world - mouth.a.xy;
        float2 down = mouth.a.zw;
        float along = dot(d, down);
        // Out to sea the plume's axis wanders, a quarter of the way out at most.
        float wander = (value_noise(float2(max(along, 0.0) / 150.0, float(m) * 7.1)) - 0.5) * RIVER_PLUME_WANDER * 2.0 * max(along, 0.0);
        float across = abs(d.x * down.y - d.y * down.x - wander);
        float half_width = mouth.b.x;
        float reach = RIVER_PLUME_LENGTH * half_width;
        float spread = half_width + RIVER_PLUME_SPREAD * max(along, 0.0);
        if (along > reach || along < -200.0 || across > spread + 3.0)
            continue;
        float fresh = 1.0 - smoothstep(0.5 * spread, spread + 3.0, across);
        if (along < 0.0)
            fresh *= 1.0 - smoothstep(150.0, 200.0, -along);
        else
        {
            fresh *= sqrt(half_width / spread) * (1.0 - smoothstep(0.4 * reach, reach, along));
            float broken = saturate(0.4 + 1.2 * value_noise(world / 90.0));
            fresh *= lerp(1.0, broken, smoothstep(0.0, 6.0 * half_width, along));
        }
        e.fresh = max(e.fresh, fresh);
        e.velocity += down * (mouth.b.y * fresh);
        // The white water a river brings, carried on out of its channel and fading.
        float spill = 1.0 - smoothstep(0.0, RIVER_SPILL * half_width, along);
        e.white = max(e.white, mouth.b.z * spill * (1.0 - smoothstep(0.5 * spread, spread + 3.0, across)));
    }
    e.fresh = saturate(e.fresh);
    return e;
}

// The stones in a river (`forge_procgen::stones`): where one breaks the surface the water parts
// around its outline at the level as a potential flow parts around a cylinder (a uniform stream
// and a doublet: `u − iv = U (1 − R²/z²)` in the stream's frame, each stone's change added), piles
// up white in front of it and trails a wake behind, widening and fading; one under the water
// only roughens the surface over it and a little way down. What else stands in the water shows
// by the water thinning against it (the contact in `river_frag_main`).
struct StoneFlow
{
    float2 velocity; // world x, z; m/s
    float white;     // the white water's share, 0..1
    float rough;     // how much rougher the surface is, 0..1
};

// Points either side of a segment whose stones reach it.
static const uint RIVER_STONE_POINTS = 3;
// A wake's spread (metres of half width a metre down) and its length in the stone's radii.
static const float RIVER_WAKE_SPREAD = 0.2;
static const float RIVER_WAKE_LENGTH = 8.0;

StoneFlow stones_at(WaterSurface* w, uint segment, float2 world, float2 velocity)
{
    StoneFlow o;
    o.velocity = velocity;
    o.white = 0.0;
    o.rough = 0.0;
    float speed = length(velocity);
    if (w[0].stone_count == 0u || speed < 1e-3)
        return o;
    uint lo = segment >= RIVER_STONE_POINTS ? segment - RIVER_STONE_POINTS : 0u;
    uint hi = min(segment + RIVER_STONE_POINTS, w[0].river_points - 1u);
    uint first = uint(w[0].rivers[lo].e.z);
    uint last = uint(w[0].rivers[hi].e.z);
    float2 down = velocity / speed;
    float2 side = float2(-down.y, down.x);
    float2 change = 0.0;
    for (uint i = first; i < last; i++)
    {
        float4 s = w[0].stones[i].a;
        float2 d = world - s.xy;
        float x = dot(d, down);
        float y = dot(d, side);
        float r2 = max(dot(d, d), 1e-4);
        float r = sqrt(r2);
        float outline = s.z;
        if (outline > 0.0)
        {
            float k = outline * outline / (r2 * r2);
            float2 local = float2(1.0 - k * (x * x - y * y), -2.0 * k * x * y);
            change += (down * (local.x - 1.0) + side * local.y) * speed;
            float front = (r - outline) / (0.3 * outline + 0.05);
            float pillow = exp(-front * front) * saturate(-x / r);
            float spread = outline + RIVER_WAKE_SPREAD * max(x, 0.0);
            float across = y / spread;
            float wake = x > 0.0 ? exp(-across * across) * exp(-x / (RIVER_WAKE_LENGTH * outline)) : 0.0;
            o.white = max(o.white, max(pillow, 0.7 * wake));
            o.rough = max(o.rough, wake);
        }
        else
        {
            float radius = s.w;
            float spread = radius + RIVER_WAKE_SPREAD * max(x, 0.0);
            float across = y / spread;
            float before = min(x, 0.0) / radius;
            float trail = exp(-across * across) * exp(-max(x, 0.0) / (0.5 * RIVER_WAKE_LENGTH * radius)) * exp(-before * before);
            o.rough = max(o.rough, 0.6 * trail);
            o.white = max(o.white, 0.3 * trail);
        }
    }
    // Never faster than twice the stream (the flanks of a cylinder's flow), which also keeps the
    // points inside a stone, hidden under it, from running away.
    float changed = length(change);
    o.velocity = velocity + (changed > speed ? change * (speed / changed) : change);
    // White water only where the stream runs fast enough to break.
    o.white *= smoothstep(0.3, 1.5, speed);
    return o;
}

// What floats in a river (#107): the water parts around each thing as around a stone breaking
// the surface (`stones_at`), the flow taken relative to it, so a thing the stream carries at
// its own speed barely stirs it and one held back or pushed through it piles up white in front
// and trails a wake. (In still water and the sea a wake is waves, which the stones' flow does
// not draw: wave particles, `docs/research/water.md`.) Its flow and its wake fade out over the
// second half of this many radii of its outline. A pixel looks at those its cell of the grid
// around the camera lists, the ones whose reach overlaps it.
static const float FLOATER_REACH = 30.0;

StoneFlow floaters_at(WaterSurface* w, float2 world, StoneFlow o)
{
    float4 grid = w[0].floater_grid;
    uint cells = uint(grid.w);
    int2 cell = int2(floor((world - grid.xy) * grid.z));
    if (any(cell < 0) || any(cell >= int(cells)))
        return o;
    uint listed = w[0].floater_cells[uint(cell.y) * cells + uint(cell.x)];
    uint first = cells * cells + (listed & 0xFFFFu);
    float2 change = 0.0;
    float fastest = 0.0;
    for (uint j = first; j < first + (listed >> 16); j++)
    {
        Floater f = w[0].floaters[w[0].floater_cells[j]];
        float2 d = world - f.a.xy;
        float outline = f.a.z;
        float reach = FLOATER_REACH * outline;
        float r2 = max(dot(d, d), 1e-4);
        if (r2 >= reach * reach)
            continue;
        float r = sqrt(r2);
        float fade = 1.0 - smoothstep(0.5, 1.0, r / reach);
        float2 relative = o.velocity - f.b.xy;
        float speed = length(relative);
        if (speed < 1e-3)
            continue;
        fastest = max(fastest, speed);
        float2 down = relative / speed;
        float2 side = float2(-down.y, down.x);
        float x = dot(d, down);
        float y = dot(d, side);
        float k = outline * outline / (r2 * r2);
        float2 local = float2(1.0 - k * (x * x - y * y), -2.0 * k * x * y);
        change += (down * (local.x - 1.0) + side * local.y) * (speed * fade);
        float front = (r - outline) / (0.3 * outline + 0.05);
        float pillow = exp(-front * front) * saturate(-x / r);
        float spread = outline + RIVER_WAKE_SPREAD * max(x, 0.0);
        float across = y / spread;
        float wake = x > 0.0 ? exp(-across * across) * exp(-x / (RIVER_WAKE_LENGTH * outline)) * fade : 0.0;
        o.white = max(o.white, max(pillow, 0.7 * wake) * smoothstep(0.3, 1.5, speed));
        o.rough = max(o.rough, wake);
    }
    // As around the stones: never faster than twice the flow past them.
    float changed = length(change);
    o.velocity += changed > fastest ? change * (fastest / changed) : change;
    return o;
}

// White water's pattern: metres a cell of its coarser octave of noise across the river, and how
// many times longer the cells are down it (the finer octave is 2.5 times finer both ways).
static const float RIVER_FOAM_CELL = 0.35;
static const float RIVER_FOAM_STREAK = 4.0;
// A step's fall breaks into chutes across the river (#122): metres across a chute, and the share
// of its white the glassiest keeps.
static const float RIVER_CHUTE = 1.3;
static const float RIVER_CHUTE_GLASS = 0.25;

// White water's pattern at `p` (metres along the river and across it): two octaves of value
// noise drawn out down the river, streaks of broken white water rather than round blobs.
float foam_streaks(float2 p)
{
    float2 q = p / (RIVER_FOAM_CELL * float2(RIVER_FOAM_STREAK, 1.0));
    return 0.6 * value_noise(q) + 0.4 * value_noise(q * 2.5 + 7.1);
}

// Fresh water's surface at a pixel, a river's or a lake's: what the shading needs besides the
// frame.
struct FreshWater
{
    float4 position; // SV_Position
    float3 relative; // camera-relative metres
    float2 flat;     // camera-relative world x, z
    float2 velocity; // the flow, world x, z; m/s (0: still water)
    float2 fall;     // the surface's own slope (a river's fall downstream), for its shading
    float2 tilt;     // the surface's slope as drawn, uncapped: the water's depth is under it
    float rough;     // added to the ripples' slopes' variance (a stone's wake)
    float coverage;  // how much of the pixel it covers, before the contact's fade
    float drape;     // how far it lies on the ground far away (0: at its level)
    float depth;     // its own depth there, which the view sees through where it lies on the ground
    float white;     // the white water's share, before its pattern
    float froth;     // how much of its pattern the white water covers, 0..1: a step's fall and the boil under it (#122)
    float2 streak;   // a river's: metres along it from its head and across it, where its white water streaks
    float speed;     // a river's own speed down it (m/s), which carries its white water
    float3 absorption; // its water's, m⁻¹
    float3 scatter;  // the light its water scatters back, per unit of irradiance
    float4 bed;      // its bed's tint at the ground's own brightness, and how much of the ground it takes
    float deepest;   // the deepest it can be under its surface, metres
    float ripples;   // how strongly its ripples stand, 1 as the rivers' (a lake's calm patches less)
};

// The shading of fresh water: the ripples carried on its flow, the sky, the sun, the mirror and
// shadow rays asked for, the bed seen along the view ray through the water to it (its own depth
// where it lies on the ground far away), the light it scatters back, white water broken up by a
// pattern carried down with the flow (finer than two pixels, its mean), and the air; blended by
// its coverage, the edge fading where the water thins to nothing against the ground, a stone or
// anything in it.
SurfaceTargets fresh_water(WaterSurface* w, FreshWater s)
{
    float2 world = s.flat + w[0].camera.xz;
    float footprint = max(max(length(ddx(s.flat)), length(ddy(s.flat))), 1e-3);
    RiverRipples ripples = river_ripples(w, s.flat, s.velocity);
    float variance = ripples.variance * (s.ripples * s.ripples) + s.rough;
    float2 slope = ripples.slope * s.ripples + s.fall;
    float3 n = normalize(float3(-slope.x, 1.0, -slope.y));
    float3 v = normalize(-s.relative);
    float a2 = clamp(0.0004 + 2.0 * variance, 0.0004, 1.0);
    WaterLight light = water_light(w, n, v, a2);
    uint2 pixel = uint2(s.position.xy);
    float2 uv = (float2(pixel) + 0.5) / float2(w[0].width, w[0].height);
    float3 under = g_sampled_images[NonUniformResourceIndex(w[0].scene_color)].Load(int3(pixel, 0)).rgb;
    // The bed: the ground under the water turned to gravel and silt at its own brightness,
    // exactly where the water is. The ground's 4 m layer map paints no river bed: a pixel
    // blends its four texels and the lookup wanders by one, so a bed's texels showed metres onto
    // the banks (#114).
    float bright = dot(under, float3(0.2126, 0.7152, 0.0722));
    under = lerp(under, bright * s.bed.rgb, s.bed.a);
    float floor_depth = g_sampled_images_r[NonUniformResourceIndex(w[0].scene_depth)].Load(int3(pixel, 0));
    float surface_distance = length(s.relative);
    float path = 1e3;
    if (floor_depth > 0.0)
    {
        float4 behind = mul(w[0].sky[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, floor_depth, 1.0));
        path = max(length(behind.xyz / behind.w) - surface_distance, 0.0);
    }
    // The water's depth along the view ray, under the surface as it lies: tilted down the
    // river by its fall. Under a level surface's plane a low view up a steep river found no
    // depth at all, and the water up the valley vanished until the camera rose (#113).
    float3 lying = normalize(float3(-s.tilt.x, 1.0, -s.tilt.y));
    float thickness = path * max(dot(v, lying), 0.0);
    float contact = smoothstep(0.0, max(RIVER_CONTACT.x, RIVER_CONTACT.y * footprint), thickness);
    // Where the view ray finds nothing within the deepest the water can be under the surface,
    // the ground is not drawn there yet (its pages still streaming in): no water over nothing.
    float there = 1.0 - smoothstep(s.deepest, 1.5 * s.deepest, thickness);
    float coverage = s.coverage * lerp(contact * there, 1.0, s.drape);
    // The light's path through the water to what is under it, along the refracted ray (as the
    // sea's); far away, where the ribbon lies on the ground, through the channel's depth.
    float fall = refracted_down(n, v);
    path = lerp(thickness, s.depth, s.drape) / fall;
    float3 transmittance = exp(-s.absorption * path);
    float3 inside = under * transmittance + s.scatter * light.lit * (1.0 - transmittance);
    float3 color = light.reflected * light.fresnel + inside * (1.0 - light.fresnel) + light.sun;
    float foam = 0.0;
    if (s.white > 0.0)
    {
        // Streaks down the river, carried at its own speed in two phases (the flow map's way,
        // along the ribbon): the flow around a stone, which the ripples follow, would draw them
        // out into smooth bands metres long over a period. Finer than two pixels the pattern
        // gives way to its mean.
        FlowPhases g = flow_phases(world, float2(s.speed, 0.0), w[0].time);
        float n0 = foam_streaks(s.streak - g.offset0);
        float n1 = foam_streaks(s.streak - g.offset1 + 13.7);
        float norm = rsqrt(g.weight0 * g.weight0 + g.weight1 * g.weight1);
        float pattern = 0.5 + ((n0 - 0.5) * g.weight0 + (n1 - 0.5) * g.weight1) * norm;
        float resolved = saturate(RIVER_FOAM_CELL / (2.0 * footprint) - 1.0);
        float cover = smoothstep(0.5 - 0.45 * s.froth, 0.72 - 0.3 * s.froth, pattern);
        foam = s.white * lerp(lerp(RIVER_FOAM_MEAN, 0.9, s.froth), cover, resolved);
    }
    color = lerp(color, 0.8 * light.lit / PI, foam * 0.8);
    float4 air = aerial_at(w[0].sky, uv, surface_distance * 0.001);
    color = color * air.a + air.rgb * light.scale;
    // Blended by the coverage, as are the rays asked for (`trace_requested` normalises the
    // direction): what the mirror ray meets and what the sun lights of the water's share.
    SurfaceTargets o;
    o.color = float4(min(color, 32768.0), coverage);
    float weight = light.fresnel * (1.0 - light.blur) * (1.0 - foam * 0.8) * air.a * coverage;
    o.request = weight >= REQUEST_MIN_WEIGHT ? float4(light.mirror, weight) : float4(0.0);
    float3 sun_share = light.sun + s.scatter * light.sun_light * (1.0 - transmittance) * (1.0 - light.fresnel);
    sun_share = lerp(sun_share, 0.8 * light.sun_light / PI, foam * 0.8) * air.a;
    sun_share = min(sun_share, o.color.rgb) * coverage;
    float brightest = max(o.color.r, max(o.color.g, o.color.b)) * coverage;
    bool shade = max(sun_share.r, max(sun_share.g, sun_share.b)) > SUN_REQUEST_MIN_SHARE * brightest;
    o.sun = shade ? float4(sun_share, 1.0) : float4(0.0);
    // Nothing of the water here: leave the pixel, its depth and its rays to what is under.
    if (coverage < 1.0 / 255.0)
        discard;
    return o;
}

[shader("fragment")]
SurfaceTargets river_frag_main(RiverOut input)
{
    WaterSurface* w = surface_push.surface;
    return fresh_water(w, river_water(w, input));
}

// A river when the camera stands low enough to be under the water (#108).
[shader("fragment")]
SurfaceTargets river_under_frag_main(RiverOut input)
{
    WaterSurface* w = surface_push.surface;
    return fresh_water_seen(w, river_water(w, input));
}

// A river's water at a pixel.
FreshWater river_water(WaterSurface* w, RiverOut input)
{
    float across = input.river.x; // metres
    float2 world = input.flat + w[0].camera.xz;
    float footprint = max(max(length(ddx(input.flat)), length(ddy(input.flat))), 1e-3);
    float half_drawn = input.flow.w;
    float half_water = input.edge.x;
    float drape = input.edge.z;
    float out_from_middle = abs(across);
    // How much of the pixel the river covers. Near, the ribbon fades out past the water's edge
    // under the banks, where the ground hides it anyway (past a confluence's rounded corner,
    // whose water it covers, #119); far away, where it lies on the ground,
    // its edges are soft over half a metre or two pixels but never more than half the half
    // width drawn, and its coverage is the river's share of the width drawn. And its fade.
    float near_cover = 1.0 - smoothstep(input.cover, input.edge.y, out_from_middle);
    float soft = min(max(RIVER_BANK, 2.0 * footprint), 0.5 * half_drawn);
    float far_cover = min(half_water / half_drawn, 1.0) * smoothstep(0.0, soft, half_drawn - out_from_middle);
    // The flow: faster in the middle than by the banks, around the stones.
    float2 down = input.flow.xy / max(length(input.flow.xy), 1e-4);
    float u = saturate(out_from_middle / max(half_water, 1e-3));
    float2 velocity = down * (input.river.w * (1.0 - 0.4 * u * u));
    StoneFlow stones = stones_at(w, input.segment, world, velocity);
    // And around what floats in it (#107).
    if (w[0].floater_grid.w > 0.0)
        stones = floaters_at(w, world, stones);
    // White water in riffles where the water falls steeply and fast, and on and behind the
    // stones. (Not along the banks: the water's edge there follows the carved ground's metre
    // triangles, and a line of foam drew them.)
    float riffles = smoothstep(0.45, 0.75, value_noise(world / RIVER_RIFFLES));
    float rapids = smoothstep(0.06, 0.2, input.flow.z) * smoothstep(1.5, 3.0, input.river.w) * riffles;
    FreshWater s;
    s.position = input.position;
    s.relative = input.relative;
    s.flat = input.flat;
    s.velocity = stones.velocity;
    s.fall = -down * min(input.flow.z, 0.3);
    s.tilt = -down * input.flow.z;
    s.rough = RIVER_WAKE_ROUGHNESS * stones.rough;
    s.coverage = input.river.y * lerp(near_cover, far_cover, drape);
    s.drape = drape;
    s.depth = input.river.z * (1.0 - u * u);
    // A step's fall white through, and its boil across the pool below, thinning out (#122);
    // across the river it pours in chutes, some glassy, some white, so the lip is no straight
    // white line.
    float chutes = smoothstep(0.2, 0.8, value_noise(world / RIVER_CHUTE + 17.3));
    float fall_white = input.foam * lerp(RIVER_CHUTE_GLASS, 1.0, chutes);
    s.white = max(max(rapids, stones.white), fall_white);
    s.froth = fall_white * fall_white;
    s.streak = float2(input.edge.w, across);
    s.speed = input.river.w;
    s.absorption = RIVER_ABSORPTION;
    s.scatter = RIVER_SCATTER;
    s.bed = RIVER_BED;
    s.deepest = input.river.z + RIVER_DEEPEST;
    s.ripples = 1.0;
    return s;
}

// ----------------------------------------------------------------- the lakes ---
//
// D-038's lakes: a level plane each at the lake's level (`forge_procgen::lake_waters`: the
// priority flood's depression and a sample more), drawn after the sea and before the rivers,
// which blend over the lakes they run into. The mask clips the plane to the lake (softened over
// a sample), and within it the ground rising through the plane draws the shore. Its water is the
// rivers' (`fresh_water`) with the lake's own colour, still but where a river runs in (its jet
// and its water, as at the sea's mouths).

// Lake water (D-038's lake type): darker than the rivers', its dissolved matter taking the blue,
// clearer of silt (m⁻¹, and the light it scatters back).
static const float3 LAKE_ABSORPTION = float3(1.0, 0.6, 1.2);
// A lake's shallows of brown silt, as the river's bed (its deeper bed is the map's mud).
static const float4 LAKE_BED = float4(1.08, 1.0, 0.82, 0.9);
static const float3 LAKE_SCATTER = float3(0.008, 0.012, 0.008);
// The wind's patches over a lake: metres across, their drift (world x, z; m/s) and how calm the
// calm ones are (a share of the ripples' slopes).
static const float LAKE_GUST_SIZE = 60.0;
static const float2 LAKE_GUST_DRIFT = float2(1.2, 0.5);
static const float LAKE_CALM = 0.15;

struct LakeOut
{
    float4 position : SV_Position;
    float3 relative : POSITION; // camera-relative metres
    float2 flat : TEXCOORD0;    // camera-relative world x, z
    nointerpolation uint lake : TEXCOORD1;
};

[shader("vertex")]
LakeOut lake_vert_main(uint id : SV_VertexID)
{
    WaterSurface* w = surface_push.surface;
    uint index = id / 6u;
    Lake lake = w[0].lakes[index];
    // The mask's box: its first sample to its last, half a sample more all round.
    float2 lo = lake.a.xy - 0.5 * lake.a.w;
    float2 hi = lake.a.xy + (float2(lake.b.xy) - 0.5) * lake.a.w;
    float2 world = lerp(lo, hi, float2(QUAD_CORNERS[id % 6u]));
    float3 relative = float3(world.x, lake.a.z, world.y) - w[0].camera.xyz;
    LakeOut o;
    o.position = mul(w[0].view_proj, float4(relative, 1.0));
    o.relative = relative;
    o.flat = relative.xz;
    o.lake = index;
    return o;
}

// Whether lake `lake`'s mask covers its sample `s`.
float lake_mask(WaterSurface* w, Lake lake, int2 s)
{
    if (any(s < 0) || any(s >= int2(lake.b.xy)))
        return 0.0;
    uint k = uint(s.y) * lake.b.x + uint(s.x);
    return float((w[0].lake_masks[lake.b.z + k / 32u] >> (k % 32u)) & 1u);
}

[shader("fragment")]
SurfaceTargets lake_frag_main(LakeOut input)
{
    WaterSurface* w = surface_push.surface;
    return fresh_water(w, lake_water(w, input));
}

// A lake when the camera stands low enough to be under the water (#108).
[shader("fragment")]
SurfaceTargets lake_under_frag_main(LakeOut input)
{
    WaterSurface* w = surface_push.surface;
    return fresh_water_seen(w, lake_water(w, input));
}

// A lake's water at a pixel.
FreshWater lake_water(WaterSurface* w, LakeOut input)
{
    Lake lake = w[0].lakes[input.lake];
    float2 world = input.flat + w[0].camera.xz;
    // The mask between its samples, softened over half of one.
    float2 g = (world - lake.a.xy) / lake.a.w;
    int2 i0 = int2(floor(g));
    float2 t = g - float2(i0);
    float mask = lerp(
        lerp(lake_mask(w, lake, i0), lake_mask(w, lake, i0 + int2(1, 0)), t.x),
        lerp(lake_mask(w, lake, i0 + int2(0, 1)), lake_mask(w, lake, i0 + int2(1, 1)), t.x),
        t.y);
    FreshWater s;
    s.position = input.position;
    s.relative = input.relative;
    s.flat = input.flat;
    // Still water but where a river runs in: its jet's flow, and its water mixing into the lake's.
    Estuary estuary = estuary_at(w, world);
    s.velocity = estuary.velocity;
    s.fall = 0.0;
    s.tilt = 0.0;
    s.rough = 0.0;
    s.coverage = smoothstep(0.25, 0.75, mask);
    s.drape = 0.0;
    s.depth = 0.0;
    s.white = 0.0;
    s.froth = 0.0;
    s.streak = 0.0;
    s.speed = 0.0;
    s.absorption = lerp(LAKE_ABSORPTION, RIVER_ABSORPTION, estuary.fresh);
    s.scatter = lerp(LAKE_SCATTER, RIVER_SCATTER, estuary.fresh);
    s.bed = lerp(LAKE_BED, RIVER_BED, estuary.fresh);
    s.deepest = asfloat(lake.b.w) + RIVER_DEEPEST;
    // Patches of calm and of wind ripples drifting over the lake (cat's paws), the river's own
    // ripples where it runs in.
    float2 gusts = (world + LAKE_GUST_DRIFT * w[0].time) / LAKE_GUST_SIZE;
    float wind = lerp(LAKE_CALM, 1.0, smoothstep(0.35, 0.7, value_noise(gusts)));
    s.ripples = lerp(wind, 1.0, estuary.fresh);
    // The waves of what moves through it (#107), carried as a lake has no fall of its own.
    float footprint = max(max(length(ddx(input.flat)), length(ddy(input.flat))), 1e-3);
    s.fall = wake_slope(w, world, footprint);
    return s;
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

// ---------------------------------------------------------- under the water ---
//
// The view from under the water (#108; `docs/research/water.md`, "Afterwards"). `water/at-camera`
// finds the water's surface at the camera: the sea's, the cascades' displacement and the shore's
// trains stepped back to the point they carry over the camera. Each pixel then tests the point it
// looks through on the near plane against that surface as a plane, so a camera at the waterline
// sees the line cross its lens. From under it the sea's surface shows Snell's window: within
// 48.6° of its normal, the sky and the scene over the water, the window's edge holding the
// horizon, and the sun through the waves; beyond, it mirrors the water under it (total internal
// reflection). `water/under` then dims what each such pixel meets along the view ray, by the same
// absorption as seen from above (Beer–Lambert), and adds the light the water scatters towards the
// camera, both lit by what reaches their depth through the water over them.

// The point a pixel looks through on the near plane (camera-relative metres; reversed-Z: depth 1).
float3 near_point(WaterSurface* w, float2 pixel)
{
    float2 uv = pixel / float2(w[0].width, w[0].height);
    float4 p = mul(w[0].sky[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0));
    return p.xyz / p.w;
}

// How far the water's surface at the camera stands over a camera-relative point near it,
// metres: above zero the point is under the water.
float under_by(WaterAtCamera at, float3 p)
{
    return at.plane.x + dot(at.plane.yz, p.xz) - p.y;
}

// The light just under the water's surface: the sky's, from all round, and the sun's, which runs
// down the sun's ray bent into the water. Both are the light on a level surface
// (`WaterLight::lit`).
struct LightBelow
{
    float3 sky;
    float3 sun;
    float3 sun_dir; // the way the sun's light runs under the water (unit, downwards)
};

LightBelow light_below(WaterSurface* w)
{
    float3 l = normalize(w[0].sun.xyz);
    LightBelow o;
    o.sky = sh_irradiance(w[0].sky_light, float3(0.0, 1.0, 0.0)) * w[0].sun_radiance.w;
    o.sun = w[0].sun_radiance.rgb * saturate(l.y);
    o.sun_dir = l.y > 0.0 ? refract(-l, float3(0.0, 1.0, 0.0), 1.0 / WATER_IOR) : float3(0.0, -1.0, 0.0);
    return o;
}

// Henyey and Greenstein's asymmetry for the water's scattering of the sun's light: mostly
// onwards, so the water glows towards the sun and is darkest looking away from it.
static const float WATER_PHASE_G = 0.8;

// The most the phase function lifts the light scattered back as seen from above: towards the
// sun it reaches 660 times, and in water as murky as the lakes', which scatter much for what
// they absorb, the glow filled the view a metre under the surface.
static const float WATER_PHASE_MAX = 30.0;

// Henyey and Greenstein's phase function at the scattering angle's cosine `c`, against its
// value at `reference`, at most `WATER_PHASE_MAX`.
float phase_against(float c, float reference)
{
    float a = 1.0 + WATER_PHASE_G * WATER_PHASE_G;
    return min(pow((a - 2.0 * WATER_PHASE_G * reference) / (a - 2.0 * WATER_PHASE_G * c), 1.5), WATER_PHASE_MAX);
}

// What reaches the camera, `depth` metres under the water, from what a ray through the water
// meets `distance` metres away along `dir` (unit, camera-relative): `seen`, lit by what reaches
// its own depth `seen_depth` through the water over it and dimmed along the ray, and the light
// the water scatters towards the camera on the way, lit by what reaches each depth (`light`,
// falling off as e^{-σ (depth - dir.y s)} at `s` along the ray). The scattering is scaled so
// that the water seen straight down from its surface is the colour it shows from above
// (`scatter` times the light on the surface); away from that view it follows the phase
// function, the sky's light running straight down and the sun's down its ray.
float3 through_water(WaterAtCamera at, LightBelow light, float3 seen, float seen_depth, float depth, float3 dir, float distance)
{
    float3 sigma = at.absorption.rgb;
    float rise = max(1.0 - dir.y, 1e-3);
    float3 x = sigma * rise * distance;
    float3 g = select(x > 1e-3, (1.0 - exp(-x)) / max(x, 1e-6), 1.0 - 0.5 * x);
    float3 source = light.sky * phase_against(dir.y, -1.0)
        + light.sun * phase_against(dot(light.sun_dir, -dir), light.sun_dir.y);
    float3 scattered = 2.0 * at.scatter.rgb * source * exp(-sigma * depth) * sigma * distance * g;
    return seen * exp(-sigma * (seen_depth + distance)) + scattered;
}

// The share of the light a water surface reflects back into the water, at `cos_i` from its
// normal on the water's side (Fresnel's equations, unpolarised): 1 past the critical angle,
// asin(1 / 1.333) = 48.6°.
float fresnel_from_water(float cos_i)
{
    float sin2_t = WATER_IOR * WATER_IOR * (1.0 - cos_i * cos_i);
    if (sin2_t >= 1.0)
        return 1.0;
    float cos_t = sqrt(1.0 - sin2_t);
    float rs = (WATER_IOR * cos_i - cos_t) / (WATER_IOR * cos_i + cos_t);
    float rp = (WATER_IOR * cos_t - cos_i) / (WATER_IOR * cos_t + cos_i);
    return 0.5 * (rs * rs + rp * rp);
}

// A water surface seen from under it, as the light leaves it towards the camera: Snell's window,
// and the mirror of the water beyond it, for the surface's normal `n` facing down into the
// water and GGX's roughness² `a2`, at the pixel `position` and the camera-relative point
// `relative`. `water/under` adds the water between.
float3 seen_from_below(WaterSurface* w, float4 position, float3 relative, float3 n, float a2, WaterAtCamera at)
{
    float3 v = normalize(-relative);
    float reflectance = fresnel_from_water(max(dot(n, v), 1e-4));
    LightBelow light = light_below(w);
    // The mirror: the light the water under the surface scatters back up along the reflected
    // ray, out of an endless depth (a wave's underside that turns the ray up mirrors the same).
    float3 r = reflect(-v, n);
    r.y = min(r.y, -1e-3);
    float3 color = reflectance * through_water(at, light, 0.0, 0.0, 0.0, normalize(r), 1e9);
    if (reflectance < 1.0)
    {
        // The window: the ray bent out of the water towards what the copy of the scene holds
        // behind the pixel (the island over the water), or the sky it meets.
        float3 t = refract(-v, n, WATER_IOR);
        int3 pixel = int3(int2(position.xy), 0);
        float3 above;
        if (g_sampled_images_r[NonUniformResourceIndex(w[0].scene_depth)].Load(pixel) > 0.0)
            above = g_sampled_images[NonUniformResourceIndex(w[0].scene_color)].Load(pixel).rgb;
        else
        {
            float4 up_table = w[0].sky_light[SH_COEFFICIENTS];
            float3 forward = w[0].sky_light[SH_COEFFICIENTS + 1].xyz;
            above = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, forward, t) * w[0].sun_radiance.w;
        }
        // The sun through the waves: GGX's lobe about its direction, a third as wide as the
        // normals' (a tilt of the surface turns the ray out by a third of the tilt).
        float tl = dot(t, normalize(w[0].sun.xyz));
        float3 sun = tl > 0.0 ? w[0].sun_radiance.rgb * ggx(tl, max(a2 / 9.0, 1e-4)) : 0.0;
        // Into the water, the light of the sky's half crowds into the window: its radiance grows
        // by the index squared.
        color += (1.0 - reflectance) * WATER_IOR * WATER_IOR * (above + sun);
    }
    return color;
}

// The sea's surface seen from under it, its foam white.
SurfaceTargets sea_from_below(WaterSurface* w, SurfaceOut input, WaterAtCamera at)
{
    ShorePoint shore = shore_point(w, input.flat);
    SeaSlopes cascades = sea_slopes(w, input.flat, shore);
    // The shore's trains, resolved to the pixel's footprint at this distance (no derivatives:
    // the quad's other pixels may have been shaded from above).
    float distance = length(input.relative);
    ShoreWaves waves = shore_waves(w, input.flat, shore, max(distance * w[0].pixel, 1e-3));
    float2 slope = cascades.slope + waves.slope;
    float3 n = normalize(float3(slope.x, -1.0, slope.y)); // facing down, into the water
    float a2 = clamp(0.0004 + 2.0 * cascades.variance, 0.0004, 1.0);
    float3 color = seen_from_below(w, input.position, input.relative, n, a2, at);
    // Foam where the waves fold and where the shore's trains break, at its pattern's mean.
    LightBelow light = light_below(w);
    float foam = max(saturate((0.45 - cascades.jacobian) * 2.0), 0.5 * saturate(1.5 * waves.foam));
    color = lerp(color, 0.8 * (light.sky + light.sun) / PI, foam * 0.8);
    SurfaceTargets o;
    o.color = float4(min(color, 32768.0), 1.0);
    o.request = 0.0;
    o.sun = 0.0;
    return o;
}

// A river's or a lake's water seen from under it, its white water at its pattern's mean, blended
// by the coverage as seen from above. Its surface lies as its fall tilts it, without the
// ripples: taking them a second time made the compiler shade the white water seen from above a
// few levels differently, and the lakes' water hides its surface from a metre down.
SurfaceTargets fresh_from_below(WaterSurface* w, FreshWater s, float coverage, WaterAtCamera at)
{
    float3 n = normalize(float3(s.fall.x, -1.0, s.fall.y)); // facing down, into the water
    float a2 = clamp(0.0004 + 2.0 * s.rough, 0.0004, 1.0);
    float3 color = seen_from_below(w, s.position, s.relative, n, a2, at);
    LightBelow light = light_below(w);
    float foam = s.white * lerp(RIVER_FOAM_MEAN, 0.9, s.froth);
    color = lerp(color, 0.8 * (light.sky + light.sun) / PI, foam * 0.8);
    SurfaceTargets o;
    o.color = float4(min(color, 32768.0), coverage);
    o.request = 0.0;
    o.sun = 0.0;
    return o;
}

// Fresh water when the camera stands low enough to be under it: shaded as seen from above
// first (the derivatives see the whole quad), then from below where the pixel looks out from
// under the water.
SurfaceTargets fresh_water_seen(WaterSurface* w, FreshWater s)
{
    SurfaceTargets o = fresh_water(w, s);
    WaterAtCamera at = w[0].at_camera[0];
    if (under_by(at, near_point(w, s.position.xy)) > 0.0)
        o = fresh_from_below(w, s, o.color.a, at);
    return o;
}

// The sea's displacement at `flat` (camera-relative world x, z) as its vertices there take it,
// `spacing` metres apart.
float3 sea_displacement(WaterSurface* w, float2 flat, float spacing)
{
    ShorePoint shore = shore_point(w, flat);
    ShoreWaves waves = shore_waves(w, flat, shore, spacing);
    return displacement_at(w, flat, spacing, shore.depth) + float3(waves.offset.x, waves.height, waves.offset.y);
}

struct AtCameraPush
{
    WaterSurface* surface;
};

[[vk::push_constant]]
AtCameraPush at_camera_push;

// The water at the camera: the sea's surface there as the finest level of its mesh draws it,
// the plane of the triangle over the camera. The waves carry each point of the level surface
// sideways, so the one they carry over the camera is found by stepping back by its
// displacement; the triangle is that of the lattice's quad around it, displaced as its vertices
// are (`surface_vert_main`), which holds the camera. The exact surface stands centimetres off the
// drawn one, and a camera between the two looked through no surface at all. A thread per corner.
groupshared float3 g_corners[4];

[shader("compute")]
[numthreads(4, 1, 1)]
void at_camera_main(uint k : SV_GroupIndex)
{
    WaterSurface* w = at_camera_push.surface;
    float s = w[0].levels[0].z;
    float2 flat = 0.0;
    for (uint i = 0; i < 4u; i++)
        flat = -sea_displacement(w, flat, s).xz;
    float2 corner = w[0].levels[0].xy + floor((flat - w[0].levels[0].xy) / s) * s;
    float2 q = corner + float2(QUAD_CORNERS[k == 3u ? 5u : k]) * s;
    float3 d = sea_displacement(w, q, s);
    g_corners[k] = float3(q.x + d.x, d.y, q.y + d.z);
    GroupMemoryBarrierWithGroupSync();
    if (k != 0u)
        return;
    float3 p[4] = { g_corners[0], g_corners[1], g_corners[2], g_corners[3] };
    // The quad's two triangles, (0, 0), (0, 1), (1, 0) and (1, 0), (0, 1), (1, 1) as (x, z): the
    // first if the camera lies on its side of their shared edge.
    float2 e = p[1].xz - p[2].xz;
    bool first = (e.x * -p[2].z - e.y * -p[2].x) * (e.x * (p[0].z - p[2].z) - e.y * (p[0].x - p[2].x)) >= 0.0;
    float3 a = first ? p[0] : p[3];
    float3 n = cross(p[1] - a, p[2] - a);
    n *= n.y < 0.0 ? -1.0 : 1.0;
    float2 slope = -n.xz / max(n.y, 1e-6);
    float h = a.y - dot(slope, a.xz);
    WaterAtCamera at;
    at.plane = float4(h - w[0].camera.y, slope, 0.0);
    at.absorption = w[0].absorption;
    at.scatter = w[0].scatter;
    // In a lake or a river (the CPU knows their levels and where they lie), its level water.
    float4 fresh = w[0].fresh;
    if (fresh.y > 0.0)
    {
        bool lake = fresh.y < 1.5;
        at.plane = float4(fresh.x - w[0].camera.y, 0.0, 0.0, 0.0);
        at.absorption = float4(lake ? LAKE_ABSORPTION : RIVER_ABSORPTION, 0.0);
        at.scatter = float4(lake ? LAKE_SCATTER : RIVER_SCATTER, 0.0);
    }
    // `water/under` runs only where the near plane reaches the water, or its waterline: the
    // plane's test is linear across it, so its corners tell.
    float under = -1e9;
    for (uint c = 0; c < 4u; c++)
    {
        float2 pixel = float2((c & 1u) != 0u ? w[0].width : 0u, (c & 2u) != 0u ? w[0].height : 0u);
        under = max(under, under_by(at, near_point(w, pixel)));
    }
    uint2 groups = (uint2(w[0].width, w[0].height) + 7u) / 8u;
    at.under_groups = uint4(under > -WATERLINE_MARGIN ? groups : uint2(0u), 1u, 0u);
    w[0].at_camera[0] = at;
}

struct UnderPush
{
    WaterSurface* surface;
    uint color;  // storage: the HDR image
    uint depth;  // sampled: the depth as drawn, the water's included
    uint width;
    uint height;
};

[[vk::push_constant]]
UnderPush under_push;

// The waterline across the lens: pixels wide, and how much it darkens; and the metres the near
// plane may stand over the water with the line still on it (its pixels are hundredths of a
// millimetre there).
static const float WATERLINE_PIXELS = 1.5;
static const float WATERLINE_DARK = 0.6;
static const float WATERLINE_MARGIN = 1e-3;

// The water between the camera and what each pixel under the water meets, and the waterline
// where the near plane crosses the surface.
[shader("compute")]
[numthreads(8, 8, 1)]
void under_main(uint3 id : SV_DispatchThreadID)
{
    UnderPush p = under_push;
    if (id.x >= p.width || id.y >= p.height)
        return;
    WaterSurface* w = p.surface;
    WaterAtCamera at = w[0].at_camera[0];
    float2 pixel = float2(id.xy) + 0.5;
    float3 near = near_point(w, pixel);
    float under = under_by(at, near);
    float across = max(abs(under_by(at, near_point(w, pixel + float2(1.0, 0.0))) - under),
                       abs(under_by(at, near_point(w, pixel + float2(0.0, 1.0))) - under));
    float line = 1.0 - smoothstep(0.5 * WATERLINE_PIXELS, WATERLINE_PIXELS, abs(under) / max(across, 1e-9));
    if (under <= 0.0 && line <= 0.0)
        return;
    float4 color = g_storage_images_rgba16f[p.color][id.xy];
    if (under > 0.0)
    {
        float depth = g_sampled_images_r[p.depth].Load(int3(id.xy, 0));
        float3 dir = normalize(near);
        float distance = 1e9;
        if (depth > 0.0)
        {
            float2 uv = pixel / float2(p.width, p.height);
            float4 hit = mul(w[0].sky[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
            distance = length(hit.xyz / hit.w);
        }
        // The water's surface, where it was drawn, is lit at the surface; the scene under the
        // water at its own depth.
        float scene = g_sampled_images_r[w[0].scene_depth].Load(int3(id.xy, 0));
        float seen_depth = depth != scene ? 0.0 : max(under - dir.y * distance, 0.0);
        color.rgb = through_water(at, light_below(w), color.rgb, seen_depth, under, dir, distance);
    }
    color.rgb *= 1.0 - WATERLINE_DARK * line;
    g_storage_images_rgba16f[p.color][id.xy] = color;
}
