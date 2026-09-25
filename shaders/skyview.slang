// The sky-view table's layout (issue #43), shared by the sky's passes (sky.slang) and the
// resolve's reflections of the sky (meshlet.slang, issue #49): the light the air and the
// planet's ground send towards the camera, per unit of sun illuminance, over 192 × 108
// directions of elevation (squashed towards the horizon) and azimuth from the sun's.
module skyview;

import bindless;

public static const uint SKY_VIEW_WIDTH = 192;
public static const uint SKY_VIEW_HEIGHT = 108;
static const float PI = 3.14159265;

// Sky-view table coordinates of a direction at `elevation` over the horizontal and
// `azimuth` from the sun's (0..π, the sky being symmetric about the sun's vertical plane):
// elevation squashed so the texels crowd around the horizon.
public float2 sky_view_uv(float elevation, float azimuth)
{
    float e = elevation / (0.5 * PI);
    float v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    float u = azimuth / PI;
    // Keep the bilinear footprint inside the table.
    float2 size = float2(SKY_VIEW_WIDTH, SKY_VIEW_HEIGHT);
    return (float2(u, v) * (size - 1.0) + 0.5) / size;
}

// The table's luminance towards the unit `direction`, in the camera's frame on the planet:
// `up`, and `forward` the horizontal towards the sun.
public float3 sky_view_luminance(uint table, float3 up, float3 forward, float3 direction)
{
    float elevation = asin(clamp(dot(direction, up), -1.0, 1.0));
    float3 flat = direction - up * dot(direction, up);
    float azimuth = dot(flat, flat) > 1e-10 ? acos(clamp(dot(normalize(flat), forward), -1.0, 1.0)) : 0.0;
    return sample_image_level(table, SAMPLER_LINEAR_CLAMP, sky_view_uv(elevation, azimuth), 0.0).rgb;
}
