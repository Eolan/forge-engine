// Procedural starfield: one full-screen triangle; stars and a faint nebula hashed from the
// per-pixel view direction, so the sky is stable under camera motion and needs no textures.

struct Push
{
    float4x4 inv_view_proj;
    float3 sun_dir;
    float exposure;
    float3 planet_dir;      // direction to the planet's centre
    float planet_angle;     // angular radius of the planet (radians); 0 = no planet
};

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

// A distant planet: a lit sphere with continents, ice caps, a terminator and an atmosphere
// rim, plus a scattered halo beyond the limb. Everything is a function of the view direction.
float3 planet(float3 dir, float3 sun, out float coverage)
{
    coverage = 0.0;
    if (push.planet_angle <= 0.0)
        return 0.0;
    float3 axis = normalize(push.planet_dir);
    float distance = 1000.0;
    float radius = distance * sin(push.planet_angle);
    float3 centre = axis * distance;
    float t = dot(centre, dir);
    float d2 = distance * distance - t * t;
    float r2 = radius * radius;
    float3 color = 0;
    if (d2 < r2 && t > 0.0)
    {
        float3 p = dir * (t - sqrt(r2 - d2));
        float3 n = normalize(p - centre);
        float ndl = dot(n, sun);
        float day = smoothstep(-0.08, 0.25, ndl);
        // Surface: continents from low-frequency noise, ice caps at the poles.
        float land = value_noise(n * 3.0 + 40.0, 21u) * 0.55 + value_noise(n * 9.0 + 5.0, 23u) * 0.3 + value_noise(n * 24.0, 29u) * 0.15;
        float land_mask = smoothstep(0.47, 0.58, land);
        float3 ocean = float3(0.07, 0.20, 0.45);
        float3 ground = lerp(float3(0.16, 0.22, 0.12), float3(0.36, 0.30, 0.20), value_noise(n * 15.0, 31u));
        float3 surface = lerp(ocean, ground, land_mask);
        float cap = smoothstep(0.80, 0.92, abs(n.y) + 0.08 * value_noise(n * 12.0, 37u));
        surface = lerp(surface, float3(0.85, 0.9, 0.95), cap);
        float clouds = smoothstep(0.50, 0.72, value_noise(n * 4.0 + 90.0, 41u) * 0.6 + value_noise(n * 11.0 + 3.0, 43u) * 0.4);
        surface = lerp(surface, float3(0.95, 0.95, 0.97), clouds * 0.7);
        // Night side: faint city lights on land, and a little scattered light from the sky.
        float lights = land_mask * (1.0 - cap) * smoothstep(0.70, 0.80, value_noise(n * 60.0, 47u) * 0.7 + value_noise(n * 9.0, 53u) * 0.3) * 0.08;
        float rim = pow(1.0 - saturate(dot(-dir, n)), 3.0);
        float3 atmosphere = float3(0.35, 0.55, 1.0) * rim * (0.35 + 0.65 * day);
        float3 lit = surface * (max(ndl, 0.0) * 1.1 + 0.03);
        color = lit + float3(1.0, 0.75, 0.45) * lights * (1.0 - day) + atmosphere;
        coverage = 1.0;
    }
    // Halo beyond the limb, stronger towards the sun.
    float d = sqrt(max(d2, 0.0));
    if (t > 0.0 && d >= radius)
    {
        float3 limb_n = normalize(dir * t - centre);
        float lit = smoothstep(-0.2, 0.4, dot(limb_n, sun));
        float halo = exp(-(d - radius) / (radius * 0.06)) * (0.25 + 0.75 * lit);
        color += float3(0.35, 0.55, 1.0) * halo * 0.6;
    }
    return color;
}

[shader("fragment")]
float4 frag_main(VertexOut input) : SV_Target
{
    float4 far = mul(push.inv_view_proj, float4(input.ndc, 0.0001, 1.0));
    float4 near = mul(push.inv_view_proj, float4(input.ndc, 1.0, 1.0));
    float3 dir = normalize(far.xyz / far.w - near.xyz / near.w);

    // Milky-way-like band and a dusty nebula from low-frequency noise.
    float band = exp(-4.0 * abs(dir.y + 0.15 * dir.x));
    float n1 = value_noise(dir * 6.0 + 3.0, 7u);
    float n2 = value_noise(dir * 14.0 + 9.0, 11u);
    float n3 = value_noise(dir * 30.0 + 1.0, 13u);
    float dust = saturate(n1 * 0.6 + n2 * 0.3 + n3 * 0.1 - 0.35) * 2.2;
    float3 nebula = band * dust * float3(0.25, 0.28, 0.42) * 0.35 + band * 0.02 * float3(0.6, 0.6, 0.8);
    float3 warm = value_noise(dir * 4.0 + 20.0, 17u) * band * float3(0.30, 0.16, 0.10) * 0.25;

    // Angular size of one pixel, from the screen-space derivatives of the direction.
    float pixel_size = max(length(ddx(dir)), length(ddy(dir)));
    float3 color = stars(dir, 160.0, 0.010, 1.0, pixel_size) * 0.9 + stars(dir, 60.0, 0.006, 2.0, pixel_size) * 1.6 + nebula * 1.8 + warm * 1.5;

    // The sun: a small bright disc with a soft halo and a wide glow.
    float3 sun = normalize(push.sun_dir);
    float s = saturate(dot(dir, sun));
    color += float3(1.0, 0.95, 0.85) * (smoothstep(0.9995, 0.9999, s) * 20.0 + pow(s, 200.0) * 0.8 + pow(s, 12.0) * 0.06 + pow(s, 3.0) * 0.01);

    // The planet covers the stars behind it.
    float coverage;
    float3 planet_color = planet(dir, sun, coverage);
    color = lerp(color, planet_color, coverage) + planet_color * (1.0 - coverage);

    return float4(color * push.exposure, 1.0);
}
