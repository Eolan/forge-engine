// A planet's atmosphere after S. Hillaire, "A Scalable and Production Ready Sky and Atmosphere
// Rendering Technique" (EGSR 2020), lifted from the `world` project. Driven by
// crates/forge-render/src/atmosphere.rs, whose CPU mirror of the medium and the transmittance
// integral the tests check (keep the two in step). Distances are in kilometres, coefficients
// per kilometre, luminance per unit of sun illuminance: callers multiply by the pre-exposed
// illuminance.
//
// Two tables, rebuilt only when the atmosphere changes (atmosphere_luts.slang): transmittance
// to the top of the atmosphere (height × zenith angle) and the multiple-scattering transfer
// (sun zenith angle × height). Seen from space, the sky pass marches each pixel's ray through
// the shell with both tables (`cross_atmosphere`, Hillaire's per-pixel path for views from
// outside the atmosphere).
module atmosphere;

import bindless;

static const float PI = 3.14159265;
// Keeps sample points just above the ground, where the parameterisations divide by heights.
public static const float GROUND_OFFSET = 0.01;

// Must match GpuPlanet in atmosphere.rs (float4 fields only).
public struct Planet
{
    // x: ground radius; y: top of the atmosphere; z: ground albedo seen by the tables.
    public float4 radii;
    // rgb: Rayleigh scattering at the ground; w: its scale height.
    public float4 rayleigh;
    // rgb: aerosol scattering at the ground; w: its scale height.
    public float4 mie_scattering;
    // rgb: aerosol absorption at the ground; w: Cornette–Shanks asymmetry g.
    public float4 mie_absorption;
    // rgb: ozone absorption at the layer's peak; w: peak altitude.
    public float4 ozone;
    // x: half-width of the tent-shaped ozone layer.
    public float4 ozone_shape;
    // xyz: the camera relative to the planet's centre (world axes).
    public float4 view_position;
};

public struct Medium
{
    public float3 rayleigh;
    public float3 mie;
    public float3 scattering;
    public float3 extinction;
};

public Medium sample_medium(Planet p, float altitude)
{
    float h = max(altitude, 0.0);
    float rayleigh_density = exp(-h / p.rayleigh.w);
    float mie_density = exp(-h / p.mie_scattering.w);
    float ozone_density = max(0.0, 1.0 - abs(altitude - p.ozone.w) / p.ozone_shape.x);
    Medium m;
    m.rayleigh = p.rayleigh.rgb * rayleigh_density;
    m.mie = p.mie_scattering.rgb * mie_density;
    m.scattering = m.rayleigh + m.mie;
    m.extinction = m.scattering + p.mie_absorption.rgb * mie_density + p.ozone.rgb * ozone_density;
    return m;
}

public float rayleigh_phase(float cos_theta)
{
    return 3.0 / (16.0 * PI) * (1.0 + cos_theta * cos_theta);
}

// Cornette–Shanks: Henyey–Greenstein with a Rayleigh-like symmetric factor (Hillaire 2020).
public float mie_phase(float g, float cos_theta)
{
    float k = 3.0 / (8.0 * PI) * (1.0 - g * g) / (2.0 + g * g);
    return k * (1.0 + cos_theta * cos_theta) / pow(max(1.0 + g * g - 2.0 * g * cos_theta, 1e-4), 1.5);
}

// Where the unit ray from `origin` crosses the sphere of `radius` around the planet's centre:
// false when it misses, else the distances in and out (`near` < 0 from inside). Worked out
// from the point of closest approach, as (r − h)(r + h): a camera twenty thousand kilometres
// out keeps metre precision where r² − h² would lose kilometres at a grazing ray.
public bool sphere_span(float3 origin, float3 direction, float radius, out float near, out float far)
{
    float along = -dot(origin, direction);
    float h = length(origin + direction * along);
    near = -1.0;
    far = -1.0;
    if (h > radius)
        return false;
    float half_chord = sqrt((radius - h) * (radius + h));
    near = along - half_chord;
    far = along + half_chord;
    return far > 0.0;
}

// The planet hides the sun from `position` (above the ground).
public bool in_planet_shadow(Planet p, float3 position, float3 sun)
{
    float along = -dot(position, sun);
    return along > 0.0 && length(position + sun * along) < p.radii.x;
}

// Transmittance table: height and zenith angle (Bruneton 2017 mapping).
public void transmittance_parameters(Planet p, float2 uv, out float r, out float mu)
{
    float bottom = p.radii.x;
    float top = p.radii.y;
    float horizon = sqrt(top * top - bottom * bottom);
    float rho = horizon * uv.y;
    r = sqrt(rho * rho + bottom * bottom);
    float d_min = top - r;
    float d_max = rho + horizon;
    float d = d_min + uv.x * (d_max - d_min);
    mu = d == 0.0 ? 1.0 : (horizon * horizon - rho * rho - d * d) / (2.0 * r * d);
    mu = clamp(mu, -1.0, 1.0);
}

public float2 transmittance_uv(Planet p, float r, float mu)
{
    float bottom = p.radii.x;
    float top = p.radii.y;
    float horizon = sqrt(top * top - bottom * bottom);
    float rho = sqrt(max(0.0, r * r - bottom * bottom));
    float discriminant = r * r * (mu * mu - 1.0) + top * top;
    float d = max(0.0, -r * mu + sqrt(max(0.0, discriminant)));
    float d_min = top - r;
    float d_max = rho + horizon;
    return float2((d - d_min) / max(d_max - d_min, 1e-6), rho / horizon);
}

// Multiple-scattering table: sun zenith cosine across, height up.
public float2 multiple_scattering_uv(Planet p, float r, float sun_cos)
{
    return float2(sun_cos * 0.5 + 0.5, saturate((r - p.radii.x) / (p.radii.y - p.radii.x)));
}

public float3 sample_table(uint image, float2 uv)
{
    return g_sampled_images[NonUniformResourceIndex(image)].SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], uv, 0.0).rgb;
}

// Share of sunlight reaching `position` (planet frame), shadowed by the planet itself.
public float3 sunlight_at(Planet p, uint transmittance, float3 position, float3 sun)
{
    float r = length(position);
    if (in_planet_shadow(p, position, sun))
        return 0.0;
    return sample_table(transmittance, transmittance_uv(p, r, dot(position / r, sun)));
}

public struct Crossing
{
    // In-scattered luminance per unit of sun illuminance.
    public float3 luminance;
    // Transmittance along the ray through the atmosphere, up to the ground if it hits it.
    public float3 transmittance;
    // Where the ray meets the ground (planet frame) when `hits_ground`.
    public float3 ground;
    public bool hits_ground;
    // The ray crosses the atmosphere at all (else nothing above applies).
    public bool crosses;
};

// The view ray from the camera (`view_position`) through the atmosphere: `steps` segments,
// denser towards the ray's lowest point (where a grazing ray meets most of the air) and, for
// rays that end on the ground, towards the ground. Single scattering from the transmittance
// table (with the planet's shadow) plus the multiple-scattering table's isotropic term; each
// segment's source is integrated exactly under its own attenuation (Hillaire 2020, §5).
public Crossing cross_atmosphere(Planet p, uint transmittance, uint multiple_scattering, float3 direction, float3 sun, uint steps)
{
    Crossing c;
    c.luminance = 0.0;
    c.transmittance = 1.0;
    c.ground = 0.0;
    c.hits_ground = false;
    c.crosses = false;
    float3 origin = p.view_position.xyz;
    float top_near, top_far;
    if (!sphere_span(origin, direction, p.radii.y, top_near, top_far))
        return c;
    float ground_near, ground_far;
    c.hits_ground = sphere_span(origin, direction, p.radii.x, ground_near, ground_far) && ground_near > 0.0;
    float start = max(top_near, 0.0);
    float end = c.hits_ground ? ground_near : top_far;
    c.crosses = true;
    // March from the entry point, which keeps the samples' precision.
    float3 entry = origin + direction * start;
    float span = end - start;
    float lowest = clamp(-dot(entry, direction), 0.0, span);
    uint first = lowest >= span - 1e-3 ? steps : (lowest <= 1e-3 ? 0 : steps / 2);
    uint second = steps - first;

    float cos_theta = dot(direction, sun);
    float rayleigh_p = rayleigh_phase(cos_theta);
    float mie_p = mie_phase(p.mie_absorption.w, cos_theta);
    float3 throughput = 1.0;
    float previous = 0.0;
    for (uint i = 1; i <= steps; i++)
    {
        float t;
        if (i <= first)
        {
            float u = 1.0 - float(i) / float(first);
            t = lowest * (1.0 - u * u);
        }
        else
        {
            float u = float(i - first) / float(second);
            t = lowest + (span - lowest) * u * u;
        }
        float dt = t - previous;
        float3 position = entry + direction * (0.5 * (previous + t));
        previous = t;
        float r = length(position);
        Medium m = sample_medium(p, r - p.radii.x);
        float3 sunlight = sunlight_at(p, transmittance, position, sun);
        float3 multiple = sample_table(multiple_scattering, multiple_scattering_uv(p, r, dot(position / r, sun)));
        float3 source = sunlight * (m.rayleigh * rayleigh_p + m.mie * mie_p) + multiple * m.scattering;
        float3 segment = exp(-m.extinction * dt);
        c.luminance += throughput * (source - source * segment) / max(m.extinction, 1e-9);
        throughput *= segment;
    }
    c.transmittance = throughput;
    c.ground = entry + direction * span;
    return c;
}

// ---- The planet seen from space: a table around its direction (issue #26) ------------------
//
// From a camera outside the atmosphere, a view ray is fixed by two numbers: its closest
// approach to the planet's centre `b` (as `sphere_span` measures it) and its azimuth `phi`
// around the direction to the planet, from the half-plane that holds the sun. The medium is
// spherically symmetric, so `phi` and `-phi` see the same light: phi runs over [0, pi]. The
// planet-view table holds `cross_atmosphere`'s luminance over (b, phi), built when the camera
// or the sun moves (atmosphere_luts.slang); a one-row table holds the transmittance, which
// does not depend on the sun. Along b the left half covers the rays that end on the ground,
// the right half those that pass above it, each packed towards the ground's edge where a
// grazing ray's path through the air changes fastest; filtering never mixes the two halves.

public static const uint PLANET_VIEW_WIDTH = 512;  // along b, half for each kind of ray
public static const uint PLANET_VIEW_HEIGHT = 256; // along phi
// Segments of the march through the shell for a camera outside it (the table's and the sky's).
public static const uint SPACE_MARCH_STEPS = 16;

// The camera's frame around the planet: `toward` the planet's centre, `across` the sun's side
// of the plane through it (any side when the sun is on that line), `third` completing it.
public struct PlanetViewFrame
{
    public float3 toward;
    public float3 across;
    public float3 third;
    public float distance;
};

public PlanetViewFrame planet_view_frame(Planet p, float3 sun)
{
    PlanetViewFrame v;
    v.distance = length(p.view_position.xyz);
    v.toward = -p.view_position.xyz / v.distance;
    float3 side = sun - v.toward * dot(sun, v.toward);
    if (dot(side, side) < 1e-8)
        side = abs(v.toward.y) < 0.9 ? cross(v.toward, float3(0.0, 1.0, 0.0)) : cross(v.toward, float3(1.0, 0.0, 0.0));
    v.across = normalize(side);
    v.third = cross(v.toward, v.across);
    return v;
}

// Texture coordinate along b: the ground half by 1 − √(1 − b / ground radius), linear at the
// disc's centre (where the ground under the ray moves linearly across the terminator) and
// packed towards its edge; the upper half by the square root of its closest approach's height,
// packed towards the ground.
public float planet_view_u(Planet p, float b)
{
    float half_texel = 0.5 / float(PLANET_VIEW_WIDTH);
    if (b < p.radii.x)
        return clamp(0.5 * (1.0 - sqrt(1.0 - b / p.radii.x)), half_texel, 0.5 - half_texel);
    float h = saturate((b - p.radii.x) / (p.radii.y - p.radii.x));
    return clamp(0.5 + 0.5 * sqrt(h), 0.5 + half_texel, 1.0 - half_texel);
}

// The b of the texel column at `u` (its centre), the inverse of `planet_view_u`.
public float planet_view_b(Planet p, float u)
{
    if (u < 0.5)
    {
        float x = 1.0 - 2.0 * u;
        return p.radii.x * (1.0 - x * x);
    }
    float h = 2.0 * u - 1.0;
    return p.radii.x + h * h * (p.radii.y - p.radii.x);
}

// The view ray with closest approach `b` and azimuth `phi`.
public float3 planet_view_direction(PlanetViewFrame v, float b, float phi)
{
    float s = b / v.distance;
    float c = sqrt(max((1.0 - s) * (1.0 + s), 0.0));
    return v.toward * c + (v.across * cos(phi) + v.third * sin(phi)) * s;
}

// Where the ray `direction` from the camera falls in the planet-view table.
public float2 planet_view_uv(Planet p, PlanetViewFrame v, float3 direction)
{
    float3 origin = p.view_position.xyz;
    float b = length(origin + direction * -dot(origin, direction));
    float phi = atan2(abs(dot(direction, v.third)), dot(direction, v.across));
    float half_texel = 0.5 / float(PLANET_VIEW_HEIGHT);
    return float2(planet_view_u(p, b), clamp(phi / PI, half_texel, 1.0 - half_texel));
}

// Light arriving on the ground at `position` with unit normal `normal`, per unit of sun
// illuminance: the sun through the atmosphere on the facing side, plus the skylight as the
// multiple-scattering table's mean luminance over the upper hemisphere (π L).
public float3 ground_illuminance(Planet p, uint transmittance, uint multiple_scattering, float3 position, float3 normal, float3 sun)
{
    float sun_cos = dot(normal, sun);
    float r = p.radii.x + GROUND_OFFSET;
    float3 direct = sample_table(transmittance, transmittance_uv(p, r, sun_cos)) * max(sun_cos, 0.0);
    float3 sky = sample_table(multiple_scattering, multiple_scattering_uv(p, r, sun_cos)) * PI;
    return direct + sky;
}
