// The atmosphere's tables (atmosphere.slang has the model), built by
// crates/forge-render/src/atmosphere.rs when the atmosphere changes: transmittance to the top
// of the atmosphere, then the multiple-scattering transfer, which samples it.
import bindless;
import atmosphere;

struct LutPush
{
    Planet* planet;
    uint output;         // storage image written
    uint transmittance;  // sampled image of the transmittance table
    uint width;
    uint height;
};

[[vk::push_constant]]
LutPush lut_push;

static const float PI = 3.14159265;

// ---- Transmittance: height × zenith angle ------------------------------------------------

static const uint TRANSMITTANCE_STEPS = 40;

[shader("compute")]
[numthreads(8, 8, 1)]
void transmittance_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= lut_push.width || id.y >= lut_push.height)
        return;
    Planet p = lut_push.planet[0];
    float2 uv = (float2(id.xy) + 0.5) / float2(lut_push.width, lut_push.height);
    float r;
    float mu;
    transmittance_parameters(p, uv, r, mu);
    float3 origin = float3(0.0, r, 0.0);
    float3 direction = float3(sqrt(saturate(1.0 - mu * mu)), mu, 0.0);
    float near, far;
    float3 optical_depth = 0.0;
    if (sphere_span(origin, direction, p.radii.y, near, far))
    {
        float dt = far / float(TRANSMITTANCE_STEPS);
        for (uint i = 0; i < TRANSMITTANCE_STEPS; i++)
        {
            float3 position = origin + direction * ((float(i) + 0.5) * dt);
            optical_depth += sample_medium(p, length(position) - p.radii.x).extinction * dt;
        }
    }
    g_storage_images_rgba16f[NonUniformResourceIndex(lut_push.output)][id.xy] = float4(exp(-optical_depth), 1.0);
}

// ---- Multiple scattering: sun zenith angle × height ----------------------------------------

// Nearest hit ahead of `origin` on the sphere of `radius`, or -1.
float hit_ahead(float3 origin, float3 direction, float radius)
{
    float near, far;
    if (!sphere_span(origin, direction, radius, near, far))
        return -1.0;
    return near >= 0.0 ? near : far;
}

struct Scattered
{
    float3 luminance;  // per unit of sun illuminance, isotropic phase
    float3 transfer;   // share of light scattered along the ray (the series' ratio)
};

// Second-order light towards `origin` along `direction` (isotropic phase), and the ground
// seen at the end of the ray lit by the sun through the atmosphere.
Scattered scatter_along(Planet p, float3 origin, float3 direction, float3 sun, uint steps)
{
    Scattered result;
    result.luminance = 0.0;
    result.transfer = 0.0;
    float bottom = p.radii.x;
    float ground = hit_ahead(origin, direction, bottom);
    float top = hit_ahead(origin, direction, p.radii.y);
    if (top < 0.0)
        return result;
    float ray_length = ground > 0.0 ? min(ground, top) : top;
    float isotropic = 1.0 / (4.0 * PI);
    float dt = ray_length / float(steps);
    float3 throughput = 1.0;
    for (uint i = 0; i < steps; i++)
    {
        float3 position = origin + direction * ((float(i) + 0.3) * dt);
        float r = length(position);
        Medium m = sample_medium(p, r - bottom);
        float3 segment = exp(-m.extinction * dt);
        float3 sunlight = sunlight_at(p, lut_push.transmittance, position + position / r * GROUND_OFFSET, sun);
        float3 source = sunlight * m.scattering * isotropic;
        float3 extinction = max(m.extinction, 1e-9);
        result.luminance += throughput * (source - source * segment) / extinction;
        result.transfer += throughput * m.scattering / extinction * (1.0 - segment);
        throughput *= segment;
    }
    if (ground > 0.0 && ground <= top)
    {
        float3 position = origin + direction * ground;
        float r = length(position);
        float sun_cos = dot(sun, position / r);
        float3 sunlight = sample_table(lut_push.transmittance, transmittance_uv(p, r, sun_cos));
        result.luminance += sunlight * throughput * saturate(sun_cos) * p.radii.z / PI;
    }
    return result;
}

static const uint SPHERE_SAMPLES_SQRT = 8;

// Second-order scattering from every direction with an isotropic phase, then the geometric
// series of the higher orders (Hillaire 2020, §5.2). A dense atmosphere pushes the transfer
// towards 1, where the series diverges: it is capped at 0.95.
[shader("compute")]
[numthreads(8, 8, 1)]
void multiple_scattering_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= lut_push.width || id.y >= lut_push.height)
        return;
    Planet p = lut_push.planet[0];
    float2 uv = (float2(id.xy) + 0.5) / float2(lut_push.width, lut_push.height);
    float sun_cos = uv.x * 2.0 - 1.0;
    float r = clamp(p.radii.x + uv.y * (p.radii.y - p.radii.x), p.radii.x + GROUND_OFFSET, p.radii.y - GROUND_OFFSET);
    float3 origin = float3(0.0, r, 0.0);
    float3 sun = float3(sqrt(saturate(1.0 - sun_cos * sun_cos)), sun_cos, 0.0);

    float3 luminance = 0.0;
    float3 transfer = 0.0;
    float weight = 1.0 / float(SPHERE_SAMPLES_SQRT * SPHERE_SAMPLES_SQRT);
    for (uint i = 0; i < SPHERE_SAMPLES_SQRT; i++)
    {
        for (uint j = 0; j < SPHERE_SAMPLES_SQRT; j++)
        {
            float theta = 2.0 * PI * (float(i) + 0.5) / float(SPHERE_SAMPLES_SQRT);
            float phi = acos(1.0 - 2.0 * (float(j) + 0.5) / float(SPHERE_SAMPLES_SQRT));
            float3 direction = float3(cos(theta) * sin(phi), cos(phi), sin(theta) * sin(phi));
            Scattered s = scatter_along(p, origin, direction, sun, 20);
            luminance += s.luminance * weight;
            transfer += s.transfer * weight;
        }
    }
    float3 psi = luminance / (1.0 - min(transfer, 0.95));
    g_storage_images_rgba16f[NonUniformResourceIndex(lut_push.output)][id.xy] = float4(psi, 1.0);
}

// ---- The planet seen from space (issue #26): luminance over (b, phi), then transmittance -----

struct PlanetViewPush
{
    float4 sun;               // xyz: towards the sun
    Planet* planet;
    uint transmittance;       // sampled image of the transmittance table
    uint multiple_scattering; // sampled image of the multiple-scattering table
    uint luminance_out;       // storage image of the planet-view table
    uint transmittance_out;   // storage image of its one-row transmittance
};

[[vk::push_constant]]
PlanetViewPush view_push;

// One texel of the planet-view table: `cross_atmosphere` along its ray, with the steps the
// sky's march takes, so that the table and the march it replaces agree at the texels. Row 0
// of the second table takes the ray's transmittance.
[shader("compute")]
[numthreads(8, 8, 1)]
void planet_view_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= PLANET_VIEW_WIDTH || id.y >= PLANET_VIEW_HEIGHT)
        return;
    Planet p = view_push.planet[0];
    float3 sun = normalize(view_push.sun.xyz);
    PlanetViewFrame frame = planet_view_frame(p, sun);
    float b = planet_view_b(p, (float(id.x) + 0.5) / float(PLANET_VIEW_WIDTH));
    float phi = PI * (float(id.y) + 0.5) / float(PLANET_VIEW_HEIGHT);
    float3 direction = planet_view_direction(frame, b, phi);
    Crossing c = cross_atmosphere(p, view_push.transmittance, view_push.multiple_scattering, direction, sun, SPACE_MARCH_STEPS);
    g_storage_images_rgba16f[NonUniformResourceIndex(view_push.luminance_out)][id.xy] = float4(c.luminance, 1.0);
    if (id.y == 0)
        g_storage_images_rgba16f[NonUniformResourceIndex(view_push.transmittance_out)][uint2(id.x, 0)] = float4(c.transmittance, 1.0);
}
