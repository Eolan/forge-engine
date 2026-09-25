// The sky seen from inside an atmosphere (issue #43), after Hillaire 2020 (§4 and §5): a
// sky-view table of the in-scattered light around the camera, an aerial-perspective volume
// of what the air adds and takes away between the camera and each depth, and a compose pass
// that draws the sky and the sun where nothing was drawn and hazes what was. Driven by
// crates/forge-render/src/sky.rs; the atmosphere and its two tables are atmosphere.slang's.
// Luminance is per unit of sun illuminance until the compose multiplies it by the frame's
// pre-exposed illuminance.
import bindless;
import atmosphere;

// Must match GpuSky in sky.rs.
struct Sky
{
    float4x4 inv_view_proj;   // clip → world (the drawing camera, jitter included)
    float4 camera;            // xyz: the camera in world metres; w: metres per world unit (1)
    float4 sun;               // xyz: towards the sun (world); w: cos of the sun disc's radius
    float4 scale;             // x: pre-exposed luminance of a unit of sun illuminance; y: the
                              // sun disc's luminance per unit illuminance (1 / solid angle);
                              // z: the aerial perspective's far distance, km; w: unused
    uint transmittance;       // sampled-image indices of the atmosphere's tables
    uint multiple_scattering;
    uint sky_view;            // sky-view table: sampled index (compose) / storage index (its pass)
    uint sky_view_storage;
    uint aerial;              // aerial-perspective atlas: sampled / storage indices
    uint aerial_storage;
    uint depth;               // the frame's depth (sampled)
    uint color;               // the frame's HDR colour (storage, read and written)
    uint width;
    uint height;
    uint2 pad;
    Planet* planet;
};

struct Push
{
    Sky* sky;
};

[[vk::push_constant]]
Push push;

static const uint SKY_VIEW_WIDTH = 192;
static const uint SKY_VIEW_HEIGHT = 108;
// The aerial-perspective volume: SLICES depth slices of SLICE × SLICE froxels, laid side by
// side in a 2-D atlas (the bindless set holds 2-D images).
static const uint SLICE = 32;
static const uint SLICES = 32;
static const float PI = 3.14159265;

// The camera's frame on the planet: up, and the horizontal towards the sun's azimuth.
void local_frame(Planet p, float3 sun, out float3 up, out float3 forward, out float3 right)
{
    up = normalize(p.view_position.xyz);
    float3 flat_sun = sun - up * dot(sun, up);
    forward = dot(flat_sun, flat_sun) > 1e-8 ? normalize(flat_sun) : (abs(up.x) < 0.9 ? normalize(cross(up, float3(1, 0, 0))) : normalize(cross(up, float3(0, 0, 1))));
    right = cross(up, forward);
}

// Sky-view table coordinates of a direction at `elevation` over the horizontal and
// `azimuth` from the sun's (0..π, the sky being symmetric about the sun's vertical plane):
// elevation squashed so the texels crowd around the horizon.
float2 sky_view_uv(float elevation, float azimuth)
{
    float e = elevation / (0.5 * PI);
    float v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    float u = azimuth / PI;
    // Keep the bilinear footprint inside the table.
    float2 size = float2(SKY_VIEW_WIDTH, SKY_VIEW_HEIGHT);
    return (float2(u, v) * (size - 1.0) + 0.5) / size;
}

float3 sky_view_direction(float2 uv, float3 up, float3 forward, float3 right)
{
    float2 size = float2(SKY_VIEW_WIDTH, SKY_VIEW_HEIGHT);
    float2 t = (uv * size - 0.5) / (size - 1.0);
    float e = (t.y - 0.5) * 2.0;
    float elevation = sign(e) * e * e * 0.5 * PI;
    float azimuth = t.x * PI;
    return up * sin(elevation) + (forward * cos(azimuth) + right * sin(azimuth)) * cos(elevation);
}

// What the air shows along `direction` from the camera, per unit of sun illuminance: the
// in-scattered light, and the planet's ground (lit by the sun and the sky, seen through the
// air) where the ray meets it.
float3 view_luminance(Planet p, uint transmittance, uint multiple_scattering, float3 direction, float3 sun, uint steps)
{
    Crossing c = cross_atmosphere(p, transmittance, multiple_scattering, direction, sun, steps);
    float3 luminance = c.luminance;
    if (c.hits_ground)
    {
        float3 n = normalize(c.ground);
        float3 light = ground_illuminance(p, transmittance, multiple_scattering, c.ground, n, sun);
        luminance += c.transmittance * light * (p.radii.z / PI);
    }
    return luminance;
}

// The sky-view table: a thread per texel, 32 segments per ray.
[shader("compute")]
[numthreads(8, 8, 1)]
void sky_view_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= SKY_VIEW_WIDTH || id.y >= SKY_VIEW_HEIGHT)
        return;
    Sky* s = push.sky;
    Planet p = s[0].planet[0];
    float3 up, forward, right;
    local_frame(p, s[0].sun.xyz, up, forward, right);
    float2 uv = (float2(id.xy) + 0.5) / float2(SKY_VIEW_WIDTH, SKY_VIEW_HEIGHT);
    float3 direction = sky_view_direction(uv, up, forward, right);
    float3 luminance = view_luminance(p, s[0].transmittance, s[0].multiple_scattering, direction, s[0].sun.xyz, 32);
    g_storage_images_rgba16f[NonUniformResourceIndex(s[0].sky_view_storage)][id.xy] = float4(luminance, 1.0);
}

// The world-space direction through the pixel at `uv` (0..1, y down).
float3 view_direction(Sky* s, float2 uv)
{
    float4 far = mul(s[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0));
    return normalize(far.xyz / far.w - s[0].camera.xyz);
}

// Depth (km) of slice boundary `k` (1..SLICES): quadratic, fine near the camera.
float slice_distance(Sky* s, float k)
{
    float t = k / float(SLICES);
    return s[0].scale.z * t * t;
}

// The aerial-perspective volume: a thread per froxel column marches the view ray slice by
// slice, storing the light gathered and the mean transmittance up to each slice's far end.
[shader("compute")]
[numthreads(8, 8, 1)]
void aerial_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= SLICE || id.y >= SLICE)
        return;
    Sky* s = push.sky;
    Planet p = s[0].planet[0];
    float3 sun = s[0].sun.xyz;
    float3 direction = view_direction(s, (float2(id.xy) + 0.5) / float(SLICE));
    float cos_theta = dot(direction, sun);
    float rayleigh_p = rayleigh_phase(cos_theta);
    float mie_p = mie_phase(p.mie_absorption.w, cos_theta);
    float3 origin = p.view_position.xyz;
    float3 luminance = 0.0;
    float3 throughput = 1.0;
    float previous = 0.0;
    for (uint k = 1; k <= SLICES; k++)
    {
        float t = slice_distance(s, float(k));
        float dt = t - previous;
        float3 position = origin + direction * (0.5 * (previous + t));
        previous = t;
        float r = length(position);
        Medium m = sample_medium(p, r - p.radii.x);
        float3 sunlight = sunlight_at(p, s[0].transmittance, position, sun);
        float3 multiple = sample_table(s[0].multiple_scattering, multiple_scattering_uv(p, r, dot(position / r, sun)));
        float3 source = sunlight * (m.rayleigh * rayleigh_p + m.mie * mie_p) + multiple * m.scattering;
        float3 segment = exp(-m.extinction * dt);
        luminance += throughput * (source - source * segment) / max(m.extinction, 1e-9);
        throughput *= segment;
        uint2 texel = uint2((k - 1) * SLICE + id.x, id.y);
        g_storage_images_rgba16f[NonUniformResourceIndex(s[0].aerial_storage)][texel] = float4(luminance, dot(throughput, float3(1.0 / 3.0)));
    }
}

// The aerial perspective at screen `uv` and `distance` km: bilinear within the two nearest
// slices, linear between them.
float4 aerial_at(Sky* s, float2 uv, float distance)
{
    float w = sqrt(saturate(distance / s[0].scale.z)) * float(SLICES) - 1.0;
    float2 inside = clamp(uv * float(SLICE), 0.5, float(SLICE) - 0.5);
    Texture2D<float4> atlas = g_sampled_images[NonUniformResourceIndex(s[0].aerial)];
    SamplerState linear = g_samplers[SAMPLER_LINEAR_CLAMP];
    float2 atlas_size = float2(SLICE * SLICES, SLICE);
    if (w <= 0.0)
    {
        // Nearer than the first slice's far end: from nothing at the camera to that slice.
        float4 first = atlas.SampleLevel(linear, inside / atlas_size, 0.0);
        float f = saturate(w + 1.0);
        return float4(first.rgb * f, lerp(1.0, first.a, f));
    }
    float k = min(floor(w), float(SLICES - 2));
    float f = saturate(w - k);
    float4 a = atlas.SampleLevel(linear, (inside + float2(k * SLICE, 0.0)) / atlas_size, 0.0);
    float4 b = atlas.SampleLevel(linear, (inside + float2((k + 1.0) * SLICE, 0.0)) / atlas_size, 0.0);
    return lerp(a, b, f);
}

// The compose pass, a thread per pixel: the sky and the sun where the depth is empty, the
// aerial perspective over everything else.
[shader("compute")]
[numthreads(8, 8, 1)]
void compose_main(uint3 id : SV_DispatchThreadID)
{
    Sky* s = push.sky;
    if (id.x >= s[0].width || id.y >= s[0].height)
        return;
    Planet p = s[0].planet[0];
    float2 uv = (float2(id.xy) + 0.5) / float2(s[0].width, s[0].height);
    float depth = g_sampled_images_r[NonUniformResourceIndex(s[0].depth)].Load(int3(id.xy, 0));
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(s[0].color)];
    float3 sun = s[0].sun.xyz;
    if (depth <= 0.0)
    {
        float3 direction = view_direction(s, uv);
        float3 up, forward, right;
        local_frame(p, sun, up, forward, right);
        float elevation = asin(clamp(dot(direction, up), -1.0, 1.0));
        float3 flat = direction - up * dot(direction, up);
        float azimuth = dot(flat, flat) > 1e-10 ? acos(clamp(dot(normalize(flat), forward), -1.0, 1.0)) : 0.0;
        float3 luminance = sample_image_level(s[0].sky_view, SAMPLER_LINEAR_CLAMP, sky_view_uv(elevation, azimuth), 0.0).rgb;
        // The sun's disc, through the air (when the ground does not hide it).
        if (dot(direction, sun) > s[0].sun.w && elevation > -0.01)
            luminance += sunlight_at(p, s[0].transmittance, p.view_position.xyz, sun) * s[0].scale.y;
        // Pre-exposed, the disc can pass fp16's 65 504 at low exposures: clamp it well below.
        color[id.xy] = float4(min(luminance * s[0].scale.x, 32768.0), 1.0);
        return;
    }
    // Reversed-Z: the pixel's world position from its depth, its distance in km.
    float4 world = mul(s[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
    float distance = length(world.xyz / world.w - s[0].camera.xyz) * 0.001 * s[0].camera.w;
    float4 air = aerial_at(s, uv, distance);
    float4 c = color[id.xy];
    color[id.xy] = float4(c.rgb * air.a + air.rgb * s[0].scale.x, c.a);
}
