// The loading screen (issues #25, #200): a hammer striking a red-hot bar on an anvil in the
// bottom right corner, stylised, while a demo prepares its scene on another thread; and at the
// bottom centre, while the shaders the cache lacks compile ahead, a bar of their share done.
// Drawn into the swapchain image by crates/forge-app/src/loading.rs, every shape a 2-D distance
// field antialiased over a pixel.

struct Push
{
    float2 size;     // the target, pixels
    float time;      // seconds since the loading started
    float progress;  // the shaders compiled ahead, 0..1 (below 0: none to compile, no bar)
};

[[vk::push_constant]]
Push push;

struct VertexOut
{
    float4 position : SV_Position;
};

[shader("vertex")]
VertexOut vert_main(uint id : SV_VertexID)
{
    // Covers the screen with one triangle: (-1,-1), (3,-1), (-1,3).
    float2 ndc = float2((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0);
    VertexOut o;
    o.position = float4(ndc, 0.0, 1.0);
    return o;
}

static const float PI = 3.14159265;

// ------------------------------------------------------------------ distance fields ---
// Signed distances in the icon's units (after Inigo Quilez's 2-D primitives).

float sd_box(float2 p, float2 half_size, float radius)
{
    float2 q = abs(p) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float sd_segment(float2 p, float2 a, float2 b, float radius)
{
    float2 pa = p - a, ba = b - a;
    float h = saturate(dot(pa, ba) / dot(ba, ba));
    return length(pa - ba * h) - radius;
}

// A trapezoid standing on the y axis: half width `r1` at y = -h, `r2` at y = +h.
float sd_trapezoid(float2 p, float r1, float r2, float h)
{
    float2 k1 = float2(r2, h);
    float2 k2 = float2(r2 - r1, 2.0 * h);
    p.x = abs(p.x);
    float2 ca = float2(p.x - min(p.x, (p.y < 0.0) ? r1 : r2), abs(p.y) - h);
    float2 cb = p - k1 + k2 * saturate(dot(k1 - p, k2) / dot(k2, k2));
    float s = (cb.x < 0.0 && ca.y < 0.0) ? -1.0 : 1.0;
    return s * sqrt(min(dot(ca, ca), dot(cb, cb)));
}

float sd_triangle(float2 p, float2 a, float2 b, float2 c)
{
    float2 e0 = b - a, e1 = c - b, e2 = a - c;
    float2 v0 = p - a, v1 = p - b, v2 = p - c;
    float2 pq0 = v0 - e0 * saturate(dot(v0, e0) / dot(e0, e0));
    float2 pq1 = v1 - e1 * saturate(dot(v1, e1) / dot(e1, e1));
    float2 pq2 = v2 - e2 * saturate(dot(v2, e2) / dot(e2, e2));
    float s = sign(e0.x * e2.y - e0.y * e2.x);
    float2 d = min(min(float2(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x)),
                       float2(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x))),
                   float2(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x)));
    return -sqrt(d.x) * sign(d.y);
}

// A smooth union over `k`: the anvil's parts flow into each other.
float smooth_min(float a, float b, float k)
{
    float h = saturate(0.5 + 0.5 * (b - a) / k);
    return lerp(b, a, h) - k * h * (1.0 - h);
}

float hash(float n)
{
    return frac(sin(n * 12.9898) * 43758.5453);
}

// ---------------------------------------------------------------------- the forge ---

// A strike every PERIOD seconds: the hammer rises slowly, falls fast, strikes at IMPACT of the
// period, bounces a little and rests.
static const float PERIOD = 1.25;
static const float IMPACT = 0.72;
// The hammer turns about the smith's hand (offscreen right), its head striking the bar.
static const float2 HAND = float2(0.80, 0.27);
static const float2 STRIKE = float2(0.13, 0.135); // where the head's face meets the bar
static const float RAISED = 1.05;                 // radians at the top of the swing

// The hammer's lift above level at phase `u` of a strike, radians.
float lift(float u)
{
    if (u < 0.08)
        return 0.0;                                        // resting on the bar
    if (u < 0.56)
    {
        float t = (u - 0.08) / 0.48;
        return RAISED * t * t * (3.0 - 2.0 * t);           // raised, easing in and out
    }
    if (u < IMPACT)
    {
        float t = (u - 0.56) / (IMPACT - 0.56);
        return RAISED * (1.0 - t * t);                      // falling faster and faster
    }
    float t = (u - IMPACT) / (1.0 - IMPACT);
    return 0.10 * sin(PI * min(t * 3.0, 1.0)) * (1.0 - t); // the bounce
}

// Distance to the anvil: its face, the horn to the left, the waist and the feet.
float anvil(float2 p)
{
    float face = sd_box(p - float2(0.02, 0.0), float2(0.38, 0.065), 0.015);
    float horn = sd_triangle(p, float2(-0.34, 0.065), float2(-0.66, 0.05), float2(-0.34, -0.10)) - 0.008;
    float waist = sd_trapezoid(p - float2(0.02, -0.15), 0.12, 0.22, 0.09);
    float feet = sd_trapezoid(p - float2(0.02, -0.29), 0.32, 0.14, 0.055);
    float d = smooth_min(face, horn, 0.03);
    d = smooth_min(d, waist, 0.04);
    return smooth_min(d, feet, 0.04);
}

// Distance to the red-hot bar on the anvil's face.
float bar(float2 p)
{
    return sd_box(p - float2(0.10, 0.093), float2(0.17, 0.028), 0.02);
}

// Distances to the hammer's handle and head, turned up by `a` about the hand.
float2 hammer(float2 p, float a)
{
    float2 along = float2(-cos(a), sin(a));   // from the hand to the head
    float2 across = float2(-along.y, along.x);
    float reach = length(HAND - (STRIKE + float2(0.0, 0.14)));
    float2 head_centre = HAND + along * reach;
    float handle = sd_segment(p, HAND, head_centre, 0.022);
    float2 q = float2(dot(p - head_centre, along), dot(p - head_centre, across));
    float head = sd_box(q, float2(0.075, 0.15), 0.025);
    return float2(handle, head);
}

// Over-blends `colour` with `alpha`.
void over(inout float3 dst, float3 colour, float alpha)
{
    dst = lerp(dst, colour, saturate(alpha));
}

// The forge in the icon's frame `p` (y up, units of the icon), `pixel` an icon unit in pixels.
float3 forge(float3 colour, float2 p, float pixel, float time)
{
    float strikes = time / PERIOD;
    float u = frac(strikes);
    // Seconds since the last strike, and which strike it was (the sparks differ each time).
    float since = (u >= IMPACT ? u - IMPACT : u + 1.0 - IMPACT) * PERIOD;
    float strike = floor(strikes - IMPACT);
    float heat = exp(-since * 5.0); // the flash of a strike, fading

    // The bar's glow on the dark around it, and the warm light it casts.
    float2 to_bar = p - float2(0.10, 0.093);
    float halo = exp(-dot(to_bar, to_bar) * 9.0);
    colour += float3(0.30, 0.09, 0.02) * halo * (0.35 + 0.8 * heat);

    // The anvil: dark steel lit from above, its face's edge catching the light, warmed by the bar.
    float d = anvil(p);
    float3 steel = lerp(float3(0.045, 0.050, 0.060), float3(0.16, 0.17, 0.19), saturate((p.y + 0.34) / 0.42));
    float edge = saturate(1.0 - abs(p.y - 0.06) / 0.018) * step(-0.34, p.x);
    steel += edge * float3(0.25, 0.25, 0.27);
    steel += float3(0.45, 0.16, 0.05) * exp(-dot(to_bar, to_bar) * 16.0) * (0.4 + heat);
    over(colour, steel, 0.5 - d / pixel);

    // The bar: orange to yellow-white where hottest, brighter at each strike.
    float b = bar(p);
    float core = saturate(1.0 - abs(to_bar.x) / 0.17);
    float3 hot = lerp(float3(0.85, 0.20, 0.03), float3(1.0, 0.75, 0.35), core * core);
    over(colour, hot * (0.8 + 0.9 * heat), 0.5 - b / pixel);

    // The hammer: a wooden handle and a steel head with a bright top edge.
    float2 h = hammer(p, lift(u));
    over(colour, float3(0.30, 0.17, 0.08), 0.5 - h.x / pixel);
    float3 head = float3(0.30, 0.31, 0.34) + float3(0.40, 0.14, 0.04) * heat * 0.6;
    over(colour, head, 0.5 - h.y / pixel);

    // The flash where the head meets the bar.
    float2 to_strike = p - STRIKE;
    colour += float3(1.0, 0.62, 0.25) * exp(-dot(to_strike, to_strike) * 60.0) * heat * 1.6;

    // The sparks: streaks thrown up and out from the strike, falling, cooling from yellow-white
    // to red as they fade.
    const uint SPARKS = 16;
    const float LIFE = 0.75;
    if (since < LIFE)
    {
        for (uint k = 0; k < SPARKS; k++)
        {
            float seed = strike * 31.0 + float(k) * 7.13;
            float angle = lerp(0.40, PI - 0.40, hash(seed));
            float speed = lerp(0.8, 1.9, hash(seed + 1.7));
            float life = LIFE * lerp(0.5, 1.0, hash(seed + 3.1));
            if (since > life)
                continue;
            float2 v = speed * float2(cos(angle), sin(angle));
            float2 g = float2(0.0, -3.5);
            float2 at = STRIKE + v * since + 0.5 * g * since * since;
            float2 moving = v + g * since;
            float s = sd_segment(p, at, at - moving * 0.022, 0.006);
            float age = since / life;
            float3 spark = lerp(float3(1.0, 0.85, 0.55), float3(0.9, 0.22, 0.04), age);
            colour += spark * saturate(0.5 - s / pixel) * (1.0 - age) * 1.4;
            colour += spark * exp(-max(s, 0.0) * 80.0) * (1.0 - age) * 0.25;
        }
    }
    return colour;
}

// The shaders' bar at the bottom centre: a dim track, the share done in the sparks' orange, a
// glint running along it.
float3 progress_bar(float3 colour, float2 pixel_at, float scale, float time)
{
    float2 centre = float2(0.5 * push.size.x, push.size.y - 0.06 * scale);
    float2 half_size = float2(0.16 * scale, 0.0035 * scale);
    float2 p = pixel_at - centre;
    float track = sd_box(p, half_size, half_size.y);
    over(colour, float3(0.05, 0.055, 0.065), 0.5 - track);
    float right = -half_size.x + 2.0 * half_size.x * saturate(push.progress);
    float fill = max(track, p.x - right);
    float along = (p.x + half_size.x) / (2.0 * half_size.x);
    float glint = exp(-pow((along - frac(time * 0.6)) * 12.0, 2.0));
    float3 orange = float3(0.95, 0.45, 0.10) * (0.85 + 0.5 * glint);
    over(colour, orange, 0.5 - fill);
    colour += float3(0.30, 0.10, 0.02) * exp(-max(fill, 0.0) / (0.006 * scale)) * 0.6;
    return colour;
}

[shader("fragment")]
float4 frag_main(VertexOut input) : SV_Target
{
    float scale = min(push.size.x, push.size.y);
    float3 colour = float3(0.0); // black, as the window before its first frame (#219)
    // The forge in the bottom right corner, a sixth of the screen's height across.
    float icon = 0.16 * scale;
    float2 corner = push.size - float2(0.14, 0.13) * scale;
    float2 p = (input.position.xy - corner) / icon;
    p.y = -p.y;
    if (all(abs(p) < float2(1.3, 1.1)))
        colour = forge(colour, p, 1.0 / icon, push.time);
    if (push.progress >= 0.0)
        colour = progress_bar(colour, input.position.xy, scale, push.time);
    return float4(colour, 1.0);
}
