// Relief inside a surface, without geometry (D-107): the march along the view through a height
// map, the shadow its bumps cast along a light, and the micro-shadows of occlusion and normal maps.
// Pure functions of their arguments: `meshlet.slang`'s resolve finds the frame they work in, and
// `crates/forge-render/src/parallax.rs` mirrors them term by term for its tests.
//
// A height map's red channel is 1 high and 0 at the relief's floor. A row's `top` is the value
// that lies on the mesh's surface: the mesh is the relief's top, nothing rises out of it, so
// nothing is drawn outside the rasterised silhouette or in front of its depth.
//
// The resolve's entries compile `-fp-mode precise`, which `lerp` escapes (#281): the blends here
// are written out.
module parallax;

import bindless;

// The most a march takes along the view, and along a light (`r.pom.steps`, `r.pom.shadow_steps`).
public static const uint RELIEF_STEPS_MOST = 64u;
public static const uint RELIEF_SHADOW_STEPS_MOST = 16u;
// The fewest a march takes along the view, looking straight at the surface.
public static const uint RELIEF_STEPS_LEAST = 4u;
// How many levels of the height map the relief fades out over (`relief_fade`).
public static const float RELIEF_FADE_LEVELS = 2.0;
// The least cosine a view or a light slides by: under it, a march would run off across the map.
public static const float RELIEF_LEAST_COSINE = 0.1;
// How much of a texel of the height map a shadow's edge is at least: a hard edge drawn through
// bilinear texels stair-steps and crawls.
public static const float RELIEF_SHADOW_MARGIN = 0.75;
// The secant's rounds after the linear search.
static const uint RELIEF_REFINE = 3u;

// What a march reads.
public struct Relief
{
    public uint image;    // the height map (bindless)
    public uint sampler;  // its row's wrapping
    public float2 at;     // the map's coordinates on the mesh's surface at this pixel
    public float2 across; // how far they move from the surface down to the relief's floor along the view
    public float top;     // the map's value that lies on the surface
    public float level;   // the level of detail it reads
};

// The relief's height at `uv`: 1 on the mesh's surface, 0 on the floor.
public float relief_height(Relief r, float2 uv)
{
    return saturate(sample_image_level(r.image, r.sampler, uv, r.level).r / r.top);
}

// How far the relief fades, 1 whole to 0 gone, by the level of detail a pixel reads (the mean of
// its two axes): from `start` over RELIEF_FADE_LEVELS. By the texels under a pixel and not by a
// distance, so it follows the upscaler's mip bias and every texture's own density.
public float relief_fade(float level, float start)
{
    float t = saturate((level - start) / RELIEF_FADE_LEVELS);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

// The steps of a march crossing `span` texels of the level it reads, at the cosine `ndv` between
// the view and the surface: a step per texel crossed, never more than the angle's share of `most`.
public uint relief_steps(float span, float ndv, uint most)
{
    uint angle = most - uint(float(most - RELIEF_STEPS_LEAST) * saturate(ndv));
    return min(uint(ceil(span)) + 1u, max(angle, RELIEF_STEPS_LEAST));
}

// How far down the relief the view ray first meets it, 0 on the surface to 1 on the floor: a
// linear search in `steps`, then the secant between the last two points. With g(s) = s + h(s) − 1,
// under the ground where g ≥ 0: g(0) ≤ 0 on the surface, and g(1) = h(1) ≥ 0 on the floor, so the
// search always ends bracketing a crossing.
public float relief_march(Relief r, uint steps)
{
    float g0 = relief_height(r, r.at) - 1.0;
    if (g0 >= 0.0)
        return 0.0;
    float s0 = 0.0;
    float s1 = 1.0;
    float g1 = 0.0;
    float stride = 1.0 / float(max(steps, 1u));
    for (uint i = 1u; i <= steps; i++)
    {
        float s = float(i) * stride;
        float g = s + relief_height(r, r.at + r.across * s) - 1.0;
        if (g >= 0.0)
        {
            s1 = s;
            g1 = g;
            break;
        }
        s0 = s;
        g0 = g;
    }
    for (uint k = 0u; k < RELIEF_REFINE; k++)
    {
        float s = s0 + (s1 - s0) * -g0 / max(g1 - g0, 1e-5);
        float g = s + relief_height(r, r.at + r.across * s) - 1.0;
        if (g >= 0.0)
        {
            s1 = s;
            g1 = g;
        }
        else
        {
            s0 = s;
            g0 = g;
        }
    }
    return s0 + (s1 - s0) * -g0 / max(g1 - g0, 1e-5);
}

// The share of a light that reaches a point `sunk` down the relief (0 to 1) at the map's
// coordinates `hit`: `climb` is how far they move climbing the whole relief towards the light,
// `ndl` the cosine of the light on the surface, `depth` the relief's depth in metres, `tan_cone`
// the light's angular radius and `margin` the least radius of its cone, metres. An occlusion march
// up the ray: the light is lost by how far the ground stands inside the ray at the deepest of
// its fixed steps, over one width for the whole ray (the light's cone at the ray's end). Fixed
// steps and no noise, so there is nothing to denoise.
public float relief_light_share(Relief r, float2 hit, float sunk, float2 climb, float ndl, float depth, float tan_cone, float margin, uint steps)
{
    if (sunk <= 0.0 || steps == 0u)
        return 1.0;
    float across_ray = depth * sqrt(saturate(1.0 - ndl * ndl));
    float width = 2.0 * max(sunk * depth / ndl * tan_cone, margin);
    float stride = 1.0 / float(steps);
    float inside = 0.0;
    for (uint i = 0u; i < steps; i++)
    {
        float climbed = sunk * (float(i) + 0.5) * stride;
        inside = max(inside, (sunk - climbed) - (1.0 - relief_height(r, hit + climb * climbed)));
        if (inside * across_ray >= width)
            return 0.0;
    }
    return 1.0 - saturate(inside * across_ray / width);
}

// The share of a direct light an occlusion map's crevice keeps at the cosine `ndl` (after Brinck and
// Maximov, "The Technical Art of Uncharted 4", 2016): nothing lost at `ao` 1 from any side, a
// crevice darkening a grazing light more than one from overhead.
public float micro_shadow(float ndl, float ao)
{
    return saturate(abs(ndl) + 2.0 * ao * ao - 1.0);
}

// The share of a direct light the mapped normal `shading` may take at the terminator of the
// interpolated normal `geometric` (Chiang, Li and Burley, "Taming the Shadow Terminator", 2019):
// a bump leaning into a light its surface turns from would otherwise catch it whole. 1 where the
// two normals agree.
public float bump_terminator(float3 geometric, float3 shading, float3 light)
{
    float ns = dot(shading, light);
    float gs = dot(geometric, shading);
    if (ns <= 0.0 || gs <= 0.0)
        return 1.0;
    float g = saturate(dot(geometric, light) / (ns * gs));
    return g * (1.0 + g - g * g);
}
