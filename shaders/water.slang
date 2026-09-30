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
    g_storage_images_rgba16f[push.slopes][id.xy] = float4(p1.y, p2.x, jacobian, 0.0);
}
