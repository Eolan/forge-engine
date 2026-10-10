// The sky seen from inside an atmosphere (issue #43), after Hillaire 2020 (§4 and §5): a
// sky-view table of the in-scattered light around the camera, an aerial-perspective volume
// of what the air adds and takes away between the camera and each depth, and a compose pass
// that draws the sky and the sun where nothing was drawn and hazes what was. Driven by
// crates/forge-render/src/sky.rs; the atmosphere and its two tables are atmosphere.slang's.
// Luminance is per unit of sun illuminance until the compose multiplies it by the frame's
// pre-exposed illuminance.
import bindless;
import atmosphere;
import sh;
import skyview;
import skyframe;

struct Push
{
    Sky* sky;
    // `irradiance_main` only (#163): the table to project and where to write, in place of the
    // frame's (`sky_view`, `irradiance`) when `out` is set: the sky with the clouds in it.
    float4* out;
    uint table;
    uint pad;
};

[[vk::push_constant]]
Push push;

static const float PI = 3.14159265;

// The camera's frame on the planet: up, and the horizontal towards the sun's azimuth.
void local_frame(Planet p, float3 sun, out float3 up, out float3 forward, out float3 right)
{
    up = normalize(p.view_position.xyz);
    sky_view_frame(up, sun, forward, right);
}

// The airglow's luminance seen at `elevation` (radians) over its zenith value: a thin layer
// about 90 km up, longer to see through towards the horizon (van Rhijn's factor).
float airglow(Planet p, float elevation)
{
    float ratio = p.radii.x / (p.radii.x + 90.0);
    float c = cos(max(elevation, 0.0));
    return 1.0 / sqrt(max(1.0 - ratio * ratio * c * c, 1e-3));
}

// What the air shows along `direction` from the camera, per unit of the reference
// illuminance: the light it scatters from the key light and the other (the night's two,
// D-046), the airglow above the horizon, and the planet's ground (lit by both and the sky,
// seen through the air) where the ray meets it.
float3 view_luminance(Sky* s, Planet p, float3 direction, float elevation, uint steps)
{
    float3 key = s[0].sun.xyz;
    float key_weight = s[0].scale.w;
    float4 other = s[0].second;
    Crossing c = cross_atmosphere_lit(p, s[0].transmittance, s[0].multiple_scattering, direction, key, key_weight, other.xyz, other.w, steps);
    float3 luminance = c.luminance;
    if (c.hits_ground)
    {
        float3 n = normalize(c.ground);
        float3 light = ground_illuminance(p, s[0].transmittance, s[0].multiple_scattering, c.ground, n, key) * key_weight;
        if (other.w > 0.0)
            light += ground_illuminance(p, s[0].transmittance, s[0].multiple_scattering, c.ground, n, other.xyz) * other.w;
        luminance += c.transmittance * light * (p.radii.z / PI);
    }
    else if (s[0].moon_light.z > 0.0)
    {
        // Airglow, a little green as the night sky's oxygen line makes it.
        luminance += c.transmittance * float3(0.85, 1.05, 0.9) * (s[0].moon_light.z * airglow(p, elevation));
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
    float elevation = asin(clamp(dot(direction, up), -1.0, 1.0));
    float3 luminance = view_luminance(s, p, direction, elevation, 32);
    g_storage_images_rgba16f[NonUniformResourceIndex(s[0].sky_view_storage)][id.xy] = float4(luminance, 1.0);
}

// The longest step of the aerial volume's march, km, and the most steps a slice takes (#220).
static const float AERIAL_STEP_KM = 2.0;
static const uint AERIAL_MAX_STEPS = 48;

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
    // The night's other light (D-046): the sun's twilight under the Moon.
    float4 other = s[0].second;
    float other_cos = dot(direction, other.xyz);
    float other_rayleigh = rayleigh_phase(other_cos);
    float other_mie = mie_phase(p.mie_absorption.w, other_cos);
    float3 origin = p.view_position.xyz;
    float3 luminance = 0.0;
    float3 throughput = 1.0;
    float previous = 0.0;
    for (uint k = 1; k <= SLICES; k++)
    {
        float t = slice_distance(s, float(k));
        // A slice longer than AERIAL_STEP_KM is marched in steps of at most that (#220): seen
        // from orbit a slice spans a hundred kilometres, and one sample at its middle lands in
        // the dense low air or above it by the slice, in rings. A shorter slice is one step,
        // computed as before.
        uint steps = clamp(uint(ceil((t - previous) / AERIAL_STEP_KM)), 1u, AERIAL_MAX_STEPS);
        float slice_start = previous;
        for (uint j = 1; j <= steps; j++)
        {
            float end = j == steps ? t : slice_start + (t - slice_start) * float(j) / float(steps);
            float dt = end - previous;
            float3 position = origin + direction * (0.5 * (previous + end));
            previous = end;
            float r = length(position);
            Medium m = sample_medium(p, r - p.radii.x);
            float3 sunlight = sunlight_at(p, s[0].transmittance, position, sun);
            float3 multiple = sample_table(s[0].multiple_scattering, multiple_scattering_uv(p, r, dot(position / r, sun)));
            float3 source = (sunlight * (m.rayleigh * rayleigh_p + m.mie * mie_p) + multiple * m.scattering) * s[0].scale.w;
            if (other.w > 0.0)
            {
                float3 light = sunlight_at(p, s[0].transmittance, position, other.xyz);
                float3 other_multiple = sample_table(s[0].multiple_scattering, multiple_scattering_uv(p, r, dot(position / r, other.xyz)));
                source += (light * (m.rayleigh * other_rayleigh + m.mie * other_mie) + other_multiple * m.scattering) * other.w;
            }
            float3 segment = exp(-m.extinction * dt);
            luminance += throughput * (source - source * segment) / max(m.extinction, 1e-9);
            throughput *= segment;
        }
        uint2 texel = uint2((k - 1) * SLICE + id.x, id.y);
        g_storage_images_rgba16f[NonUniformResourceIndex(s[0].aerial_storage)][texel] = float4(luminance, dot(throughput, float3(1.0 / 3.0)));
    }
}

// ---- The night's sky (D-046, issue #164; crates/forge-render/src/night.rs) -------------------

// One star of the night's list: its direction (celestial frame) and illuminance above the
// air (lux), its colour of unit luminance.
struct Star
{
    float4 direction;
    float4 color;
};

// The star cells' side per face of the cube round the sky (`STAR_CELLS` in night.rs).
static const uint STAR_CELLS = 64;
// How far round a star its cells reach, radians (`STAR_REACH`).
static const float STAR_REACH = 0.008;

// The cube cell of a direction, as night.rs's `star_cell`.
uint star_cell(float3 d)
{
    float3 a = abs(d);
    uint face;
    float u, v;
    if (a.x >= a.y && a.x >= a.z)
    {
        face = d.x > 0.0 ? 0u : 1u;
        u = d.y / a.x;
        v = d.z / a.x;
    }
    else if (a.y >= a.z)
    {
        face = d.y > 0.0 ? 2u : 3u;
        u = d.x / a.y;
        v = d.z / a.y;
    }
    else
    {
        face = d.z > 0.0 ? 4u : 5u;
        u = d.x / a.z;
        v = d.y / a.z;
    }
    uint n = STAR_CELLS;
    uint x = min(uint((u + 1.0) * 0.5 * float(n)), n - 1);
    uint y = min(uint((v + 1.0) * 0.5 * float(n)), n - 1);
    return face * n * n + y * n + x;
}

float hash3(int3 c)
{
    uint h = uint(c.x) * 73856093u ^ uint(c.y) * 19349663u ^ uint(c.z) * 83492791u;
    h ^= h >> 13;
    h *= 0x5bd1e995u;
    h ^= h >> 15;
    return float(h & 0xFFFFFFu) / 16777216.0;
}

float value_noise(float3 x)
{
    float3 i = floor(x);
    float3 f = x - i;
    f = f * f * (3.0 - 2.0 * f);
    int3 c = int3(i);
    float a = lerp(lerp(hash3(c), hash3(c + int3(1, 0, 0)), f.x), lerp(hash3(c + int3(0, 1, 0)), hash3(c + int3(1, 1, 0)), f.x), f.y);
    float b = lerp(lerp(hash3(c + int3(0, 0, 1)), hash3(c + int3(1, 0, 1)), f.x), lerp(hash3(c + int3(0, 1, 1)), hash3(c + int3(1, 1, 1)), f.x), f.y);
    return lerp(a, b, f.z);
}

// The Milky Way's luminance (cd/m²) towards the celestial direction `c`: a band about the
// galactic plane, brighter towards its centre, mottled, split by a dark rift (its dust).
float3 milky_way(float3 c)
{
    // J2000 equatorial to galactic (the IAU rotation), rows.
    float3 g = float3(dot(float3(-0.0548756, -0.8734371, -0.4838350), c),
                      dot(float3(0.4941094, -0.4448296, 0.7469822), c),
                      dot(float3(-0.8676661, -0.1980764, 0.4559838), c));
    float b = asin(clamp(g.z, -1.0, 1.0));
    float l = atan2(g.y, g.x);
    float band = exp(-(b * b) / (0.14 * 0.14));
    float centre = 0.35 + 0.65 * exp(-(l * l) / (0.9 * 0.9));
    float mottle = 0.45 + 0.7 * value_noise(g * 9.0) + 0.35 * value_noise(g * 23.0);
    float rift = 1.0 - 0.7 * exp(-(b - 0.02) * (b - 0.02) / (0.035 * 0.035)) * exp(-(l * l) / (1.4 * 1.4)) * value_noise(g * 15.0 + 4.0) * 1.6;
    return float3(1.0, 0.94, 0.84) * (3e-4 * band * centre * mottle * saturate(rift));
}

// The stars towards the world `direction` (cd/m²): each star of the pixel's cell a Gaussian
// about a pixel wide (`pixel` radians) that keeps its illuminance.
float3 stars_toward(Sky* s, float3 direction, float pixel, out float3 celestial)
{
    celestial = float3(dot(s[0].stars_x.xyz, direction), dot(s[0].stars_y.xyz, direction), dot(s[0].stars_z.xyz, direction));
    if (s[0].stars == nullptr)
        return 0.0;
    float sigma = clamp(pixel * 0.75, 1e-6, STAR_REACH / 3.0);
    float inverse = 0.5 / (sigma * sigma);
    float norm = 1.0 / (2.0 * PI * sigma * sigma);
    uint cell = star_cell(celestial);
    uint begin = s[0].star_cells[cell];
    uint end = s[0].star_cells[cell + 1];
    uint list = 6u * STAR_CELLS * STAR_CELLS + 1u;
    float3 sum = 0.0;
    for (uint i = begin; i < end; i++)
    {
        uint k = s[0].star_cells[list + i];
        float4 d = s[0].stars[2 * k];
        float3 offset = celestial - d.xyz;
        float g = exp(-dot(offset, offset) * inverse);
        if (g > 1e-4)
            sum += s[0].stars[2 * k + 1].rgb * (d.w * g * norm);
    }
    return sum;
}

// The night's sky beyond the air at `direction` (`elevation` over the horizon), per unit of
// the reference illuminance: the Moon's disc lit by phase (Lommel-Seeliger, normalised to its
// measured illuminance in night.rs), the stars and the Milky Way, all through the air.
float3 night_sky(Sky* s, Planet p, float3 direction, float elevation)
{
    float k = s[0].moon_light.x;
    float gain = s[0].moon_light.w;
    if (elevation < -0.01 || (k <= 0.0 && gain <= 0.0))
        return 0.0;
    float3 through = sunlight_at(p, s[0].transmittance, p.view_position.xyz, direction);
    float3 moon = s[0].moon.xyz;
    float cos_radius = s[0].moon.w;
    if (k > 0.0 && dot(direction, moon) > cos_radius)
    {
        // The disc's frame: towards us, its north up the celestial pole, across.
        float sin_radius = sqrt(max(1.0 - cos_radius * cos_radius, 1e-12));
        float3 pole = s[0].stars_z.xyz;
        float3 toward = -moon;
        float3 north = normalize(pole - moon * dot(pole, moon));
        float3 east = cross(north, toward);
        float3 offset = direction - moon * dot(direction, moon);
        float x = dot(offset, east) / sin_radius;
        float y = dot(offset, north) / sin_radius;
        float z = sqrt(max(1.0 - x * x - y * y, 0.0));
        float3 n = x * east + y * north + z * toward;
        float mu0 = dot(n, s[0].sun_disc.xyz);
        float albedo = 1.0;
        // The disc's diameter in pixels.
        float across = 2.0 * sin_radius / max(s[0].stars_x.w, 1e-9);
        if (s[0].moon_albedo != 0xFFFFFFFFu)
        {
            // The near side's longitude and latitude, the map's centre facing the Earth.
            float2 uv = float2(0.5 + atan2(x, z) / (2.0 * PI), 0.5 - asin(clamp(y, -1.0, 1.0)) / PI);
            // The level the disc's size asks for: its diameter spans half the map's 512 texels
            // of longitude.
            float level = max(log2(256.0 / max(across, 1e-3)), 0.0);
            albedo = sample_image_level(s[0].moon_albedo, SAMPLER_LINEAR_CLAMP, uv, level).r;
        }
        // Light from the Earth on its dark side, a little: the earthshine.
        float lit = mu0 > 0.0 ? mu0 / (mu0 + z) : 0.0;
        // Once the disc fills enough of the view (from 8 to 64 pixels across: a telephoto), the
        // eye adapts to it on its own (local adaptation), so its maria still read under a
        // night's exposure that would burn it white: the whole disc scaled by its mean
        // pre-exposed luminance's ratio to mid grey, raised to 0.3, which keeps the contrast
        // inside it. Smaller, it stays the bright point that glows. By day the Moon is about
        // as bright as the sky round it and stays as it is.
        float mean = dot(through, float3(0.2126, 0.7152, 0.0722)) * k * 0.5 * s[0].scale.x;
        float power = lerp(1.0, 0.3, smoothstep(8.0, 64.0, across));
        float local = mean > 0.18 ? 0.18 * pow(mean / 0.18, power) / mean : 1.0;
        return through * (k * albedo * (lit + 0.0015) * local);
    }
    if (gain <= 0.0)
        return 0.0;
    float3 celestial;
    float3 light = stars_toward(s, direction, s[0].stars_x.w, celestial);
    if (s[0].stars_y.w > 0.0)
        light += milky_way(celestial);
    return through * light * gain;
}

// Steps of a pixel's own march beyond the aerial volume (#220).
static const uint MARCH_BEYOND_STEPS = 32;

// The light the air gathers along `direction` from `t0` to `t1` km from the camera, and the
// mean transmittance (`a`), as the aerial volume gathers it, in MARCH_BEYOND_STEPS steps closer
// together towards `t1`: seen from high up, the air the ray crosses is densest near the ground
// it ends on. The samples stay over the ground.
float4 march_air(Sky* s, Planet p, float3 direction, float t0, float t1)
{
    float3 sun = s[0].sun.xyz;
    float cos_theta = dot(direction, sun);
    float rayleigh_p = rayleigh_phase(cos_theta);
    float mie_p = mie_phase(p.mie_absorption.w, cos_theta);
    float3 origin = p.view_position.xyz;
    float3 luminance = 0.0;
    float3 throughput = 1.0;
    float previous = t0;
    for (uint j = 1; j <= MARCH_BEYOND_STEPS; j++)
    {
        float u = 1.0 - float(j) / float(MARCH_BEYOND_STEPS);
        float t = t1 - (t1 - t0) * u * u;
        float dt = t - previous;
        float3 position = origin + direction * (0.5 * (previous + t));
        previous = t;
        float r = max(length(position), p.radii.x + 0.001);
        position = normalize(position) * r;
        Medium m = sample_medium(p, r - p.radii.x);
        float3 sunlight = sunlight_at(p, s[0].transmittance, position, sun);
        float3 multiple = sample_table(s[0].multiple_scattering, multiple_scattering_uv(p, r, dot(position / r, sun)));
        float3 source = (sunlight * (m.rayleigh * rayleigh_p + m.mie * mie_p) + multiple * m.scattering) * s[0].scale.w;
        float3 segment = exp(-m.extinction * dt);
        luminance += throughput * (source - source * segment) / max(m.extinction, 1e-9);
        throughput *= segment;
    }
    return float4(luminance, dot(throughput, float3(1.0 / 3.0)));
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
        float3 luminance = sky_view_luminance(s[0].sky_view, up, forward, direction);
        // The sun's disc, through the air (when the ground does not hide it).
        float3 sun_disc = s[0].sun_disc.xyz;
        if (dot(direction, sun_disc) > s[0].sun.w && elevation > -0.01)
            luminance += sunlight_at(p, s[0].transmittance, p.view_position.xyz, sun_disc) * (s[0].scale.y * s[0].sun_disc.w);
        // The Moon and the stars (D-046).
        luminance += night_sky(s, p, direction, elevation);
        float3 seen = luminance * s[0].scale.x;
        // The cloud layer over the sky and the sun (#145): what they let through, and their
        // own light (pre-exposed).
        if (s[0].clouds != 0xFFFFFFFFu)
        {
            float4 c = sample_image_level(s[0].clouds, SAMPLER_LINEAR_CLAMP, uv, 0.0);
            seen = seen * c.a + c.rgb;
        }
        // Pre-exposed, the disc can pass fp16's 65 504 at low exposures: clamp it well below.
        color[id.xy] = float4(min(seen, 32768.0), 1.0);
        return;
    }
    // Reversed-Z: the pixel's world position from its depth, its distance in km.
    float4 world = mul(s[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
    float distance = length(world.xyz / world.w - s[0].camera.xyz) * 0.001 * s[0].camera.w;
    float4 air;
    float far = s[0].scale.z;
    if (s[0].march_beyond != 0u && distance > far)
    {
        // Beyond the volume (#220): its light to its far end, then the rest of the way marched
        // for this pixel alone, dimmed by what the volume let through.
        float4 near = aerial_over(s, uv, far);
        float4 rest = march_air(s, p, view_direction(s, uv), far, distance);
        air = float4(near.rgb + near.a * rest.rgb, near.a * rest.a);
    }
    else
        air = aerial_over(s, uv, distance);
    air = thinned(air, s[0].haze);
    float4 c = color[id.xy];
    color[id.xy] = float4(c.rgb * air.a + air.rgb * s[0].scale.x, c.a);
}

// The sky's irradiance (issue #47): the sky-view table, which holds the light the air and the
// planet's ground send towards the camera from every direction, projected on the first nine
// spherical harmonics and convolved with the clamped cosine (sh.slang). One group of
// IRRADIANCE_THREADS threads, each summing IRRADIANCE_SAMPLES directions of a Fibonacci
// sphere, then a reduction in a fixed order: the result does not depend on timing. The
// resolve reads it as the light a surface receives from the sky and the ground, per unit of
// sun illuminance (the sun's disc is not in the table: the resolve adds the sun itself).
static const uint IRRADIANCE_THREADS = 256;
static const uint IRRADIANCE_SAMPLES = 16;
groupshared float3 g_irradiance[IRRADIANCE_THREADS];

[shader("compute")]
[numthreads(256, 1, 1)]
void irradiance_main(uint3 id : SV_GroupThreadID)
{
    Sky* s = push.sky;
    bool clouded = push.out != nullptr;
    uint table = clouded ? push.table : s[0].sky_view;
    float4* irradiance = clouded ? push.out : s[0].irradiance;
    Planet p = s[0].planet[0];
    float3 up, forward, right;
    local_frame(p, s[0].sun.xyz, up, forward, right);
    const uint count = IRRADIANCE_THREADS * IRRADIANCE_SAMPLES;
    float fill = s[0].moon_light.y;
    float3 moon_fill = fill > 0.0 ? sunlight_at(p, s[0].transmittance, p.view_position.xyz, s[0].moon.xyz) : 0.0;
    float3 sum[SH_COEFFICIENTS];
    [unroll]
    for (uint k = 0; k < SH_COEFFICIENTS; k++)
        sum[k] = float3(0.0);
    for (uint j = 0; j < IRRADIANCE_SAMPLES; j++)
    {
        uint i = id.x + j * IRRADIANCE_THREADS;
        float z = 1.0 - (2.0 * float(i) + 1.0) / float(count);
        float r = sqrt(max(1.0 - z * z, 0.0));
        float phi = float(i) * 2.39996323; // the golden angle
        float3 d = float3(r * cos(phi), z, r * sin(phi));
        float3 luminance = sky_view_luminance(table, up, forward, d);
        // `--moon-fill` (D-046): the Moon's light as a broad lobe round it, (n + 1) / 2π cosⁿ
        // carrying its illuminance, in the sky's light only (not drawn, not reflected).
        if (fill > 0.0)
            luminance += moon_fill * (fill * 5.0 / (2.0 * PI) * pow(max(dot(d, s[0].moon.xyz), 0.0), 4.0));
        float b[9];
        sh_basis(d, b);
        [unroll]
        for (uint k = 0; k < SH_COEFFICIENTS; k++)
            sum[k] += luminance * b[k];
    }
    for (uint k = 0; k < SH_COEFFICIENTS; k++)
    {
        g_irradiance[id.x] = sum[k];
        GroupMemoryBarrierWithGroupSync();
        for (uint stride = IRRADIANCE_THREADS / 2; stride > 0; stride /= 2)
        {
            if (id.x < stride)
                g_irradiance[id.x] += g_irradiance[id.x + stride];
            GroupMemoryBarrierWithGroupSync();
        }
        if (id.x == 0)
            irradiance[k] = float4(g_irradiance[0] * (4.0 * PI / float(count)) * sh_cosine_band(k), 0.0);
        GroupMemoryBarrierWithGroupSync();
    }
    // After the coefficients, what the resolve needs to look the table up for reflections
    // (issue #49): the camera's frame on the planet and the table's sampled index.
    if (id.x == 0)
    {
        // The index as a float value (exact below 2^24): its bits would be a denormal, which
        // the GPU may flush to zero.
        irradiance[SH_COEFFICIENTS] = float4(up, float(table));
        irradiance[SH_COEFFICIENTS + 1] = float4(forward, 0.0);
    }
}
