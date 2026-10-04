// The sky-view table's layout (issue #43), shared by the sky's passes (sky.slang) and the
// resolve's reflections of the sky (meshlet.slang, issue #49): the light the air and the
// planet's ground send towards the camera, per unit of the frame's reference illuminance, over
// 384 × 108 directions of elevation (squashed towards the horizon) and azimuth from the key
// light's. The azimuth runs round the whole circle (D-046): with two lights (the Moon and the
// sun's twilight) the sky is no longer symmetric about the key's vertical plane.
module skyview;

import bindless;

public static const uint SKY_VIEW_WIDTH = 384;
public static const uint SKY_VIEW_HEIGHT = 108;
static const float PI = 3.14159265;

// Sky-view table coordinates of a direction at `elevation` over the horizontal and
// `azimuth` from the key light's (-π..π, positive towards `right`): elevation squashed so the
// texels crowd around the horizon, the key's azimuth in the middle.
public float2 sky_view_uv(float elevation, float azimuth)
{
    float e = elevation / (0.5 * PI);
    float v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    float u = azimuth / (2.0 * PI) + 0.5;
    // Keep the bilinear footprint inside the table; its two ends are the same azimuth.
    float2 size = float2(SKY_VIEW_WIDTH, SKY_VIEW_HEIGHT);
    return (float2(u, v) * (size - 1.0) + 0.5) / size;
}

// The table's coordinates of the unit `direction`, in the camera's frame on the planet: `up`,
// and `forward` the horizontal towards the key light.
public float2 sky_view_uv_of(float3 up, float3 forward, float3 direction)
{
    float elevation = asin(clamp(dot(direction, up), -1.0, 1.0));
    float3 right = cross(up, forward);
    float3 flat = direction - up * dot(direction, up);
    float azimuth = dot(flat, flat) > 1e-10 ? atan2(dot(flat, right), dot(flat, forward)) : 0.0;
    return sky_view_uv(elevation, azimuth);
}

// The table's luminance towards the unit `direction`, in the camera's frame on the planet.
public float3 sky_view_luminance(uint table, float3 up, float3 forward, float3 direction)
{
    return sample_image_level(table, SAMPLER_LINEAR_CLAMP, sky_view_uv_of(up, forward, direction), 0.0).rgb;
}

// The camera's frame on the planet the table is laid out in: `up` at the camera, `forward` the
// horizontal towards the key light (any horizontal when it stands overhead), `right` across.
public void sky_view_frame(float3 up, float3 sun, out float3 forward, out float3 right)
{
    float3 flat_sun = sun - up * dot(sun, up);
    forward = dot(flat_sun, flat_sun) > 1e-8 ? normalize(flat_sun) : (abs(up.x) < 0.9 ? normalize(cross(up, float3(1, 0, 0))) : normalize(cross(up, float3(0, 0, 1))));
    right = cross(up, forward);
}

// The direction of the table's coordinates `uv` (the inverse of `sky_view_uv`).
public float3 sky_view_direction(float2 uv, float3 up, float3 forward, float3 right)
{
    float2 size = float2(SKY_VIEW_WIDTH, SKY_VIEW_HEIGHT);
    float2 t = (uv * size - 0.5) / (size - 1.0);
    float e = (t.y - 0.5) * 2.0;
    float elevation = sign(e) * e * e * 0.5 * PI;
    float azimuth = (t.x - 0.5) * 2.0 * PI;
    return up * sin(elevation) + (forward * cos(azimuth) + right * sin(azimuth)) * cos(elevation);
}
