// An impostor's layout (D-075 step 4, crates/forge-render/src/impostor.rs): the upper hemisphere
// unfolded into FRAMES × FRAMES frames, each baked from the direction at its middle. The
// atlases: the base colour premultiplied by its coverage, which is its alpha; the normal
// (× 0.5 + 0.5); the leaves' Fresnel weight and the depth, the surface's distance from the
// middle towards the frame's camera in radii; each premultiplied by the same coverage.
module impostor_layout;

public static const uint FRAMES = 8;

// The layout's direction at `uv` (`impostor::direction`).
public float3 layout_direction(float2 uv)
{
    float2 p = uv * 2.0 - 1.0;
    float x = 0.5 * (p.x + p.y);
    float z = 0.5 * (p.x - p.y);
    return normalize(float3(x, max(1.0 - abs(x) - abs(z), 0.0), z));
}

// Where `d` falls in the layout (`impostor::layout_point`): below the horizon, at it.
public float2 layout_point(float3 d)
{
    d.y = max(d.y, 0.0);
    d /= max(abs(d.x) + d.y + abs(d.z), 1e-12);
    return float2(d.x + d.z, d.x - d.z) * 0.5 + 0.5;
}

// A frame's picture axes (`impostor::frame_axes`).
public void frame_axes(float3 d, out float3 right, out float3 up)
{
    float3 r = cross(float3(0.0, 1.0, 0.0), d);
    right = dot(r, r) > 1e-8 ? normalize(r) : float3(1.0, 0.0, 0.0);
    up = cross(d, right);
}

// The direction frame `frame` (column, row) was baked from.
public float3 frame_direction(float2 frame)
{
    return layout_direction((frame + 0.5) / float(FRAMES));
}

// The three frames around direction `d` and their weights, summing to 1 (`impostor::frames_for`).
public void frames_around(float3 d, out float2 frames[3], out float weights[3])
{
    float2 grid = layout_point(d) * float(FRAMES) - 0.5;
    float2 corner = clamp(floor(grid), float2(0.0), float2(float(FRAMES) - 2.0));
    float2 t = saturate(grid - corner);
    bool lower = t.x + t.y <= 1.0;
    frames[0] = corner + (lower ? float2(0.0, 0.0) : float2(1.0, 1.0));
    frames[1] = corner + (lower ? float2(1.0, 0.0) : float2(0.0, 1.0));
    frames[2] = corner + (lower ? float2(0.0, 1.0) : float2(1.0, 0.0));
    weights[0] = lower ? 1.0 - t.x - t.y : t.x + t.y - 1.0;
    weights[1] = lower ? t.x : 1.0 - t.x;
    weights[2] = lower ? t.y : 1.0 - t.y;
}

