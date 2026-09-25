// Procedural starfield: one full-screen triangle; stars and a faint nebula hashed from the
// per-pixel view direction, so the sky is stable under camera motion and needs no textures.
// Units (crates/forge-render/src/starfield.rs): the sun's disc is physical (its illuminance
// over its solid angle); the planet's ground is lit albedo × E / π through its atmosphere,
// whose light is physical too (atmosphere.slang); the stars, nebula, sun glow and city lights
// are authored in units of a white Lambertian surface facing the sun. Everything is written
// pre-exposed and clamped far below the fp16 limit.
import atmosphere;

struct Push
{
    float4x4 inv_view_proj;
    float3 sun_dir;
    float luminance_unit;       // pre-exposed luminance of a white Lambertian surface facing the sun
    float3 planet_dir;          // from the camera to the planet's centre
    float planet_cos;           // cosine of the angular radius of the top of its atmosphere
    float sun_disc;             // pre-exposed luminance of the sun's disc
    float sun_angle;            // angular radius of the sun's disc (radians)
    Planet* planet;             // the planet and its atmosphere; null = none
    uint transmittance;         // the atmosphere's tables (sampled images)
    uint multiple_scattering;
    uint planet_view;           // the planet-view table (issue #26); 0xFFFFFFFF: march every pixel instead
    uint planet_view_row;       // its transmittance row; when marching, the segments per ray (issue #73)
};

// Pre-exposed values above this are white on any curve; fp16 overflows at 65504.
static const float MAX_PRE_EXPOSED = 16384.0;

[[vk::push_constant]]
Push push;

struct VertexOut
{
    float4 position : SV_Position;
    float2 ndc : NDC;
};

[shader("vertex")]
VertexOut vert_main(uint id : SV_VertexID)
{
    // Covers the screen with one triangle: (-1,-1), (3,-1), (-1,3).
    float2 ndc = float2((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0);
    VertexOut o;
    o.position = float4(ndc, 0.0, 1.0);
    o.ndc = ndc;
    return o;
}

uint hash_u32(uint n)
{
    n ^= n >> 16;
    n *= 0x7feb352du;
    n ^= n >> 15;
    n *= 0x846ca68bu;
    n ^= n >> 16;
    return n;
}

float hash3(int3 p, uint seed)
{
    uint n = hash_u32(uint(p.x) * 73856093u ^ uint(p.y) * 19349663u ^ uint(p.z) * 83492791u ^ seed);
    return float(n & 0xFFFFFF) / 16777215.0;
}

float value_noise(float3 p, uint seed)
{
    float3 b = floor(p);
    float3 f = p - b;
    f = f * f * (3.0 - 2.0 * f);
    int3 i = int3(b);
    float x00 = lerp(hash3(i, seed), hash3(i + int3(1, 0, 0), seed), f.x);
    float x10 = lerp(hash3(i + int3(0, 1, 0), seed), hash3(i + int3(1, 1, 0), seed), f.x);
    float x01 = lerp(hash3(i + int3(0, 0, 1), seed), hash3(i + int3(1, 0, 1), seed), f.x);
    float x11 = lerp(hash3(i + int3(0, 1, 1), seed), hash3(i + int3(1, 1, 1), seed), f.x);
    return lerp(lerp(x00, x10, f.y), lerp(x01, x11, f.y), f.z);
}

// Stars: a lattice of cells on the unit sphere's bounding cube; a few percent of cells hold
// one star with a random sub-cell position, brightness and tint. `pixel_size` is the angular
// footprint of one pixel: a star's glow is never narrower than a pixel, so it does not twinkle
// as the camera moves (energy is conserved by scaling the brightness down instead).
float3 stars(float3 dir, float cells, float density, float seed_offset, float pixel_size)
{
    float min_size = pixel_size * pixel_size * 0.9;
    float3 p = dir * cells;
    float3 cell = floor(p);
    float3 color = 0;
    [unroll]
    for (int dz = -1; dz <= 1; dz++)
    [unroll]
    for (int dy = -1; dy <= 1; dy++)
    [unroll]
    for (int dx = -1; dx <= 1; dx++)
    {
        int3 c = int3(cell) + int3(dx, dy, dz);
        uint h = hash_u32(uint(c.x) * 73856093u ^ uint(c.y) * 19349663u ^ uint(c.z) * 83492791u ^ uint(seed_offset));
        if (float(h & 0xFFFF) / 65535.0 > density)
            continue;
        float3 offset = float3(float((h >> 16) & 0xFF), float((h >> 8) & 0xFF), float(h & 0xFF)) / 255.0;
        float3 star = (float3(c) + offset) / cells;
        float3 star_dir = normalize(star);
        float d = 1.0 - dot(dir, star_dir);
        float size = 6.0e-7 + 5.0e-6 * pow(float((h >> 4) & 0xFF) / 255.0, 6.0);
        float shown = max(size, min_size);
        float glow = exp(-d / shown) * (size / shown);
        float temp = float((h >> 12) & 0xFF) / 255.0;
        float3 tint = lerp(float3(1.0, 0.75, 0.55), float3(0.7, 0.8, 1.0), temp);
        color += glow * tint * (0.4 + 1.6 * pow(float((h >> 20) & 0xFF) / 255.0, 3.0));
    }
    return color;
}

// The planet's ground from its unit normal: oceans, continents, ice caps and a cloud deck as
// albedo, and city lights on land for the night side (authored).
struct Ground
{
    float3 albedo;
    float lights;
};

Ground planet_ground(float3 n)
{
    float land = value_noise(n * 3.0 + 40.0, 21u) * 0.55 + value_noise(n * 9.0 + 5.0, 23u) * 0.3 + value_noise(n * 24.0, 29u) * 0.15;
    float land_mask = smoothstep(0.47, 0.58, land);
    float3 ocean = float3(0.015, 0.04, 0.09);
    float3 soil = lerp(float3(0.07, 0.10, 0.04), float3(0.30, 0.24, 0.15), value_noise(n * 15.0, 31u));
    float3 albedo = lerp(ocean, soil, land_mask);
    float cap = smoothstep(0.80, 0.92, abs(n.y) + 0.08 * value_noise(n * 12.0, 37u));
    albedo = lerp(albedo, float3(0.75, 0.80, 0.85), cap);
    float clouds = smoothstep(0.50, 0.72, value_noise(n * 4.0 + 90.0, 41u) * 0.6 + value_noise(n * 11.0 + 3.0, 43u) * 0.4);
    albedo = lerp(albedo, float3(0.80, 0.80, 0.82), clouds * 0.8);
    Ground g;
    g.albedo = albedo;
    g.lights = land_mask * (1.0 - cap) * (1.0 - clouds) * smoothstep(0.70, 0.80, value_noise(n * 60.0, 47u) * 0.7 + value_noise(n * 9.0, 53u) * 0.3) * 0.02;
    return g;
}

[shader("fragment")]
float4 frag_main(VertexOut input) : SV_Target
{
    float4 far = mul(push.inv_view_proj, float4(input.ndc, 0.0001, 1.0));
    float4 near = mul(push.inv_view_proj, float4(input.ndc, 1.0, 1.0));
    float3 dir = normalize(far.xyz / far.w - near.xyz / near.w);
    // Angular size of one pixel, from the screen-space derivatives of the direction (before
    // any branch: the whole quad takes them).
    float pixel_size = max(length(ddx(dir)), length(ddy(dir)));
    float3 sun = normalize(push.sun_dir);

    // The planet first: where its ground hides the sky, the stars are not worked out at all.
    bool planet = push.planet != nullptr && dot(dir, push.planet_dir) > push.planet_cos;
    Crossing c;
    c.crosses = false;
    c.hits_ground = false;
    if (planet)
    {
        Planet p = push.planet[0];
        if (push.planet_view != 0xFFFFFFFFu)
        {
            // Two fetches from the planet-view table (issue #26) instead of the march; where
            // the ray lands is worked out here, for the ground's own lighting.
            float2 uv = planet_view_uv(p, planet_view_frame(p, sun), dir);
            c.luminance = sample_table(push.planet_view, uv);
            c.transmittance = sample_table(push.planet_view_row, float2(uv.x, 0.5));
            float near_t, far_t;
            c.crosses = sphere_span(p.view_position.xyz, dir, p.radii.y, near_t, far_t);
            c.hits_ground = sphere_span(p.view_position.xyz, dir, p.radii.x, near_t, far_t) && near_t > 0.0;
            c.ground = p.view_position.xyz + dir * near_t;
        }
        else
            c = cross_atmosphere(p, push.transmittance, push.multiple_scattering, dir, sun, push.planet_view_row);
    }

    float3 color = 0.0;
    if (c.crosses && c.hits_ground)
    {
        Planet p = push.planet[0];
        float3 n = normalize(c.ground);
        Ground g = planet_ground(n);
        float3 light = ground_illuminance(p, push.transmittance, push.multiple_scattering, c.ground, n, sun);
        float night = 1.0 - smoothstep(-0.1, 0.05, dot(n, sun));
        color = (g.albedo * light + float3(1.0, 0.75, 0.45) * g.lights * night) * push.luminance_unit;
    }
    else
    {
        // Milky-way-like band and a dusty nebula from low-frequency noise.
        float band = exp(-4.0 * abs(dir.y + 0.15 * dir.x));
        float n1 = value_noise(dir * 6.0 + 3.0, 7u);
        float n2 = value_noise(dir * 14.0 + 9.0, 11u);
        float n3 = value_noise(dir * 30.0 + 1.0, 13u);
        float dust = saturate(n1 * 0.6 + n2 * 0.3 + n3 * 0.1 - 0.35) * 2.2;
        float3 nebula = band * dust * float3(0.25, 0.28, 0.42) * 0.35 + band * 0.02 * float3(0.6, 0.6, 0.8);
        float3 warm = value_noise(dir * 4.0 + 20.0, 17u) * band * float3(0.30, 0.16, 0.10) * 0.25;
        color = stars(dir, 160.0, 0.010, 1.0, pixel_size) * 0.9 + stars(dir, 60.0, 0.006, 2.0, pixel_size) * 1.6 + nebula * 1.8 + warm * 1.5;

        // The sun: a glow (authored, like the stars) around a physical disc whose edge is
        // antialiased over one pixel. The angle comes from the cross product, which keeps its
        // precision where acos(dot) would not.
        float s = saturate(dot(dir, sun));
        color += float3(1.0, 0.95, 0.85) * (pow(s, 200.0) * 0.8 + pow(s, 12.0) * 0.06 + pow(s, 3.0) * 0.01);
        float sun_offset = asin(min(length(cross(dir, sun)), 1.0));
        float disc = dot(dir, sun) > 0.0 ? saturate((push.sun_angle - sun_offset) / max(pixel_size, 1e-6) + 0.5) : 0.0;
        color = color * push.luminance_unit + float3(1.0, 0.95, 0.85) * disc * push.sun_disc;
    }

    // The planet's atmosphere dims and reddens what is seen through it (its ground, or the
    // stars and the sun) and adds the light it scatters.
    if (c.crosses)
    {
        // Luminance per unit of sun illuminance, times the pre-exposed illuminance (π × the unit).
        color = color * c.transmittance + c.luminance * (push.luminance_unit * 3.14159265);
    }

    return float4(min(color, MAX_PRE_EXPOSED), 1.0);
}
