// The ground sky's frame block (`GpuSky` in crates/forge-render/src/sky.rs) and the lookups
// shared by what reads it: sky.slang's passes, and the water surface (water.slang, issue
// #105), which is drawn after the sky's compose and hazes itself.
module skyframe;

import bindless;
import atmosphere;

// Must match GpuSky in sky.rs.
public struct Sky
{
    public float4x4 inv_view_proj;   // clip → world (the drawing camera, jitter included)
    public float4 camera;            // xyz: the camera in world metres; w: metres per world unit (1)
    public float4 sun;               // xyz: towards the key light, the sun or at night the Moon
                                     // (world, D-046); w: cos of the sun disc's radius
    public float4 scale;             // x: pre-exposed luminance of a unit of the reference
                                     // illuminance (the sun's by day); y: the sun disc's luminance
                                     // per unit illuminance (1 / solid angle); z: the aerial
                                     // perspective's far distance, km; w: the key's illuminance
                                     // over the reference
    public float4 sun_disc;          // xyz: towards the sun; w: its illuminance over the reference
    public float4 second;            // xyz: towards the other light; w: its weight (0: none)
    public float4 moon;              // xyz: towards the Moon; w: cos of its disc's radius
    public float4 moon_light;        // x: the Moon's disc luminance per unit Lommel-Seeliger
                                     // share, over the reference (0: no disc); y: its soft fill's
                                     // weight (`--moon-fill`, 0: none); z: airglow at the zenith
                                     // over the reference; w: the stars' gain over the reference
    public float4 stars_x;           // rows of the stars' frame from world; stars_x.w: the
    public float4 stars_y;           // pixel's angle (radians), stars_y.w: 1 to draw the Milky
    public float4 stars_z;           // Way, stars_z.w: unused
    public uint transmittance;       // sampled-image indices of the atmosphere's tables
    public uint multiple_scattering;
    public uint sky_view;            // sky-view table: sampled index (compose) / storage index (its pass)
    public uint sky_view_storage;
    public uint aerial;              // aerial-perspective atlas: sampled / storage indices
    public uint aerial_storage;
    public uint depth;               // the frame's depth (sampled)
    public uint color;               // the frame's HDR colour (storage, read and written)
    public uint width;
    public uint height;
    public uint clouds;              // sampled: the cloud layer over the sky (#145), or ~0: none
    public uint pad;
    public Planet* planet;
    public float4* irradiance;       // the sky's irradiance, SH_COEFFICIENTS coefficients, then up and forward (irradiance_main)
    public float4* stars;            // the night's stars, two float4 each (`Star` in sky.slang), or null
    public uint* star_cells;         // their cells (crates/forge-render/src/night.rs, `bin_stars`)
    public uint moon_albedo;         // sampled: the Moon's albedo map, or ~0: a plain disc
    public uint pad1;
    public uint pad2;
    public uint pad3;
};

// The aerial-perspective volume: SLICES depth slices of SLICE × SLICE froxels, laid side by
// side in a 2-D atlas (the bindless set holds 2-D images).
public static const uint SLICE = 32;
public static const uint SLICES = 32;

// The world-space direction through the pixel at `uv` (0..1, y down).
public float3 view_direction(Sky* s, float2 uv)
{
    float4 far = mul(s[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0));
    return normalize(far.xyz / far.w - s[0].camera.xyz);
}

// Depth (km) of slice boundary `k` (1..SLICES): quadratic, fine near the camera.
public float slice_distance(Sky* s, float k)
{
    float t = k / float(SLICES);
    return s[0].scale.z * t * t;
}

// The air nearer than this (km) hazes nothing (#171). The volume lights every froxel by the
// open sky and the sun, unshadowed: indoors, under an exposure metered on the shade, the air of
// a 30 m arcade read as a blue fog at sunrise. The atmosphere's haze starts beyond, as Unreal's
// sky atmosphere does by default (its aerial perspective start depth, 0.1 km); the first
// 100 m of a clear day's air at sea level dim what lies beyond by about 0.2 %.
static const float AERIAL_START_KM = 0.1;

// The aerial perspective at screen `uv` and `distance` km, over the air beyond
// `AERIAL_START_KM`: the volume's light and transmittance from there, so the haze grows from
// nothing at that depth with no edge.
public float4 aerial_at(Sky* s, float2 uv, float distance)
{
    if (distance <= AERIAL_START_KM)
        return float4(0.0, 0.0, 0.0, 1.0);
    // Along a ray, the light to d is the light to the start plus what the rest gathers, dimmed
    // by the start's transmittance; the transmittances multiply.
    float4 start = aerial_volume(s, uv, AERIAL_START_KM);
    float4 whole = aerial_volume(s, uv, distance);
    float through = max(start.a, 1e-4);
    return float4(max(whole.rgb - start.rgb, 0.0) / through, saturate(whole.a / through));
}

// The aerial-perspective volume at screen `uv` and `distance` km, from the camera: bilinear
// within the two nearest slices, linear between them.
float4 aerial_volume(Sky* s, float2 uv, float distance)
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
