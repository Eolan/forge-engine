// Clear glass drawn over the resolved scene (D-065's part 4, the owner's choice of 2026-10-18).
// Driven by crates/forge-render/src/glass.rs.
//
// The panes of the houses whose rooms are built (the village's) leave the visibility pass: the
// resolve shades the room behind each window, and these passes lay the glass over it, after the
// sky's compose and the water, seen from the street or from the room:
// - `glass/requests`: the nearest pane at each pixel asks for its mirror ray, as the water does
//   (`trace_requested` in meshlet.rs traces it against the scene, which holds no glass): its
//   direction and the weight of what the ray meets in place of the sky the pane reflected. Its
//   own depth, tested and written, keeps the nearest pane's ray where two panes overlap.
// - `glass/draw`: every pane over what is behind it, premultiplied: the room dimmed by what the
//   glass lets through, the sky in the mirror direction added by the Fresnel term, hazed as the
//   compose hazes the rest. A lit room's glow fades in with distance, where the frame's lamps
//   (`lamps::nearest`) no longer light the room behind it: by its room lamp's share the frame
//   leaves unlit (#284), or by distance for a pane without one.
// Both test the frame's depth themselves (the glass writes none: TAA reprojects the room through
// it by the room's own depth).

import bindless;
import skyframe;
import skyview;
import sh;

// Below this weight a mirror ray changes too little to trace (the water's).
static const float REQUEST_MIN_WEIGHT = 0.001;

// A window's reveal over its opening's width: seen aslant, the reveal's sides fill the pane and
// less of the room shows (#284: a lit room's glow from afar, matched against the rooms as drawn).
static const float REVEAL_DEPTH = 0.5;

// Each pane its own condition (#339, step 1 of research/glass-credibility.md): real panes are
// never in one plane, never alike and never clean, and a row of identical mirrors read as "too
// perfect". From a hash of the pane:
// - a tilt of up to PANE_TILT radians each way (a sash not quite square, a sealed unit bowed);
// - roller waves (toughened glass), a ripple of up to PANE_WAVE radians every PANE_WAVE_LENGTH
//   metres across and up, faded out where a pixel covers a quarter of a wave so it never
//   shimmers;
// - its reflectance at normal incidence: the look's for single glazing, PANE_DOUBLE_F0 times it
//   for double glazing's four surfaces (two panes in three), each give or take a tenth;
// - a haze, the film a pane gathers between cleanings: up to PANE_HAZE of what passes scattered
//   into a veil the sky lights, most panes near clean.
// The mirror rays follow the bent normal (`glass/requests` and `glass/draw` share it).
static const float PANE_TILT = 0.006;
static const float PANE_WAVE = 0.003;
static const float PANE_WAVE_LENGTH = 0.9;
static const float PANE_DOUBLE_F0 = 2.5;
static const float PANE_HAZE = 0.08;
static const float PI = 3.14159265;

// Four numbers in [0, 1) from a pane's index and a salt (PCG's output permutation).
float4 pane_hash(uint pane, uint salt)
{
    float4 o;
    uint state = pane * 747796405u + salt * 2891336453u + 1u;
    [unroll]
    for (uint k = 0; k < 4; k++)
    {
        state = state * 747796405u + 2891336453u;
        uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
        o[k] = float((word >> 22u) ^ word) * (1.0 / 4294967296.0);
    }
    return o;
}

// Must match GpuGlassVertex in glass.rs: a triangle's corner, three to a triangle.
struct GlassVertex
{
    float4 position; // xyz: the scene's frame, metres
    float4 normal;   // xyz: unit; either side is drawn; w: its pane, an index into
                     // `GlassFrame::panes` (as bits, #325)
    float4 glow;     // rgb: a lit room's light through the pane, cd/m² (0: a dark room); w: its
                     // room's lamp, an index into `GlassFrame::unlit` (as bits; ~0: none, #284)
};

// Must match GpuGlassFrame in glass.rs.
struct GlassFrame
{
    float4x4 view_proj;   // camera-relative, jitter included
    float4 camera;        // xyz: the camera in the scene's frame
    float4 look;          // x: the share the glass lets through besides what it reflects; y: its
                          // reflectance at normal incidence (F0); z, w: the distances over which
                          // the glow fades in, metres
    float sky_scale;      // pre-exposed luminance of a unit of the sky's table
    float exposure;       // pre-exposed per cd/m²
    uint depth;           // the frame's depth (sampled)
    uint unlit_count;     // the lamps `unlit` holds
    float width;
    float height;
    float4* panes;        // each pane's transform, three rows of a 3x4 matrix (glass.rs, `set_pane`, #325)
    GlassVertex* vertices;
    Sky* sky;             // the sky's frame block, for the haze
    float4* sky_light;    // the sky's irradiance, then its table's up and forward (sky.slang)
    float* unlit;         // the share of each lamp's light the frame leaves unlit (glass.rs, `set_unlit`)
};

struct GlassPush
{
    GlassFrame* frame;
};

[[vk::push_constant]]
GlassPush glass_push;

struct GlassVaryings
{
    float4 position : SV_Position;
    float3 rel : REL;       // camera-relative, metres
    float3 normal : NORMAL;
    float4 glow : GLOW;     // rgb: the glow, its lamp's unlit share in; w: 1 where a lamp drives it
    nointerpolation uint pane : PANE; // its pane, for its condition (#339)
};

[shader("vertex")]
GlassVaryings vert_main(uint vertex : SV_VertexID)
{
    GlassFrame* f = glass_push.frame;
    GlassVertex v = f[0].vertices[vertex];
    // Where its pane is now (#325): moved, or hidden by a zero transform (every corner at one
    // point, so the triangle covers nothing).
    uint pane = asuint(v.normal.w) * 3;
    float4 r0 = f[0].panes[pane];
    float4 r1 = f[0].panes[pane + 1];
    float4 r2 = f[0].panes[pane + 2];
    float4 p = float4(v.position.xyz, 1.0);
    float3 at = float3(dot(r0, p), dot(r1, p), dot(r2, p));
    float3 rel = at - f[0].camera.xyz;
    GlassVaryings o;
    o.position = mul(f[0].view_proj, float4(rel, 1.0));
    o.rel = rel;
    o.normal = float3(dot(r0.xyz, v.normal.xyz), dot(r1.xyz, v.normal.xyz), dot(r2.xyz, v.normal.xyz));
    o.pane = asuint(v.normal.w);
    // A pane whose room has a lamp glows with the share of its light the frame's lamps leave
    // unlit (#284), at any distance; the others fade in by distance.
    uint lamp = asuint(v.glow.w);
    o.glow = float4(v.glow.rgb, 0.0);
    if (lamp < f[0].unlit_count)
        o.glow = float4(v.glow.rgb * f[0].unlit[lamp], 1.0);
    return o;
}

// Whether the frame's surface at the pixel stands in front of the pane (reversed-Z: nearer is
// greater): a wall, a frame's bar, a figure in the street.
bool hidden(GlassFrame* f, float4 position)
{
    float scene = g_sampled_images_r[NonUniformResourceIndex(f[0].depth)].Load(int3(int2(position.xy), 0));
    return position.z < scene;
}

// The light on a pane, seen from the camera.
struct GlassLight
{
    float3 mirror;   // the mirror direction
    float fresnel;   // Schlick's
    float through;   // the share of what is behind that passes
    float distance;  // metres from the camera
    float room;      // the share of the pane the room behind shows through, past its reveal
    float4 air;      // the haze between the camera and the pane
    float3 normal;   // the pane's, bent by its condition, towards the camera
    float haze;      // the share of what passes its haze scatters (#339)
};

GlassLight glass_light(GlassFrame* f, GlassVaryings input)
{
    GlassLight o;
    o.distance = length(input.rel);
    float3 v = -input.rel / max(o.distance, 1e-4);
    // The pane's condition (#339): its tilt and roller waves bend its normal in its own plane.
    float3 n = normalize(input.normal);
    float4 h = pane_hash(input.pane, 0u);
    float4 w = pane_hash(input.pane, 1u);
    float3 up = abs(n.y) < 0.95 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0);
    float3 across = normalize(cross(up, n));
    float3 rise = cross(n, across);
    float3 world = input.rel + f[0].camera.xyz;
    float footprint = max(length(ddx(input.rel)), length(ddy(input.rel)));
    float fade = saturate(1.0 - footprint / (0.25 * PANE_WAVE_LENGTH));
    float2 waves = sin(float2(dot(world, across), dot(world, rise)) * (2.0 * PI / PANE_WAVE_LENGTH) + w.xy * (2.0 * PI));
    float2 lean = (h.xy * 2.0 - 1.0) * PANE_TILT + waves * w.zw * (PANE_WAVE * fade);
    n = normalize(n + across * lean.x + rise * lean.y);
    // Either side: the street's or the room's.
    if (dot(n, v) < 0.0)
        n = -n;
    o.normal = n;
    float nv = saturate(dot(n, v));
    float f0 = f[0].look.y * (h.z < 1.0 / 3.0 ? 1.0 : PANE_DOUBLE_F0) * (0.9 + 0.2 * w.x);
    o.fresnel = f0 + (1.0 - f0) * pow(1.0 - nv, 5.0);
    o.mirror = normalize(reflect(-v, n));
    o.haze = PANE_HAZE * h.w * h.w;
    o.through = (1.0 - o.fresnel) * f[0].look.x;
    // The share of the pane through which the room shows rather than its reveal's sides.
    float tan_view = sqrt(max(1.0 - nv * nv, 0.0)) / max(nv, 1e-3);
    o.room = saturate(1.0 - REVEAL_DEPTH * tan_view);
    float2 uv = saturate(input.position.xy / float2(f[0].width, f[0].height));
    o.air = aerial_at(f[0].sky, uv, o.distance * 0.001);
    return o;
}

struct RequestTargets
{
    float4 mirror : SV_Target0;
    float4 sun : SV_Target1;
};

[shader("fragment")]
RequestTargets request_main(GlassVaryings input)
{
    GlassFrame* f = glass_push.frame;
    if (hidden(f, input.position))
        discard;
    GlassLight l = glass_light(f, input);
    // What the ray meets replaces the sky `draw_main` reflected, by the same weight. No sun's
    // highlight on the glass, so no shadow ray.
    float weight = l.fresnel * l.air.a;
    RequestTargets o;
    o.mirror = weight >= REQUEST_MIN_WEIGHT ? float4(l.mirror, weight) : float4(0.0);
    o.sun = float4(0.0);
    return o;
}

[shader("fragment")]
float4 draw_main(GlassVaryings input) : SV_Target0
{
    GlassFrame* f = glass_push.frame;
    if (hidden(f, input.position))
        discard;
    GlassLight l = glass_light(f, input);
    float4 up_table = f[0].sky_light[SH_COEFFICIENTS];
    float3 forward = f[0].sky_light[SH_COEFFICIENTS + 1].xyz;
    float3 sky = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, forward, l.mirror) * f[0].sky_scale;
    // The pane's haze (#339): what it scatters of what passes is taken from the room behind and
    // given back as a veil the sky lights, as a white diffuser facing out would be.
    float through = l.through * (1.0 - l.haze);
    float alpha = 1.0 - through;
    float3 veil = (l.haze * l.through / PI) * sh_irradiance(f[0].sky_light, l.normal) * f[0].sky_scale;
    float fade = input.glow.w > 0.5 ? 1.0 : smoothstep(f[0].look.z, f[0].look.w, l.distance);
    float3 glow = input.glow.rgb * (f[0].exposure * fade * l.room);
    // Over what is behind, by what the pane stops: its reflection, its veil and the glow, hazed,
    // and the haze in front of it that the blend would dim with the room.
    float3 colour = (l.fresnel * sky + veil + glow) * l.air.a + l.air.rgb * (f[0].sky_scale * alpha);
    return float4(min(colour, 32768.0), alpha);
}
