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
//   (`lamps::nearest`) no longer light the room behind it.
// Both test the frame's depth themselves (the glass writes none: TAA reprojects the room through
// it by the room's own depth).

import bindless;
import skyframe;
import skyview;
import sh;

// Below this weight a mirror ray changes too little to trace (the water's).
static const float REQUEST_MIN_WEIGHT = 0.001;

// Must match GpuGlassVertex in glass.rs: a triangle's corner, three to a triangle.
struct GlassVertex
{
    float4 position; // xyz: the scene's frame, metres
    float4 normal;   // xyz: unit; either side is drawn
    float4 glow;     // rgb: a lit room's light through the pane, cd/m² (0: a dark room)
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
    uint pad;
    float width;
    float height;
    uint pad2;
    uint pad3;
    GlassVertex* vertices;
    Sky* sky;             // the sky's frame block, for the haze
    float4* sky_light;    // the sky's irradiance, then its table's up and forward (sky.slang)
    uint64_t pad4;
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
    float3 glow : GLOW;
};

[shader("vertex")]
GlassVaryings vert_main(uint vertex : SV_VertexID)
{
    GlassFrame* f = glass_push.frame;
    GlassVertex v = f[0].vertices[vertex];
    float3 rel = v.position.xyz - f[0].camera.xyz;
    GlassVaryings o;
    o.position = mul(f[0].view_proj, float4(rel, 1.0));
    o.rel = rel;
    o.normal = v.normal.xyz;
    o.glow = v.glow.rgb;
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
    float4 air;      // the haze between the camera and the pane
};

GlassLight glass_light(GlassFrame* f, GlassVaryings input)
{
    GlassLight o;
    o.distance = length(input.rel);
    float3 v = -input.rel / max(o.distance, 1e-4);
    // Either side: the street's or the room's.
    float3 n = normalize(input.normal);
    if (dot(n, v) < 0.0)
        n = -n;
    float nv = saturate(dot(n, v));
    float f0 = f[0].look.y;
    o.fresnel = f0 + (1.0 - f0) * pow(1.0 - nv, 5.0);
    o.mirror = normalize(reflect(-v, n));
    o.through = (1.0 - o.fresnel) * f[0].look.x;
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
    float alpha = 1.0 - l.through;
    float fade = smoothstep(f[0].look.z, f[0].look.w, l.distance);
    float3 glow = input.glow * (f[0].exposure * fade);
    // Over what is behind, by what the pane stops: its reflection and the glow, hazed, and the
    // haze in front of it that the blend would dim with the room.
    float3 colour = (l.fresnel * sky + glow) * l.air.a + l.air.rgb * (f[0].sky_scale * alpha);
    return float4(min(colour, 32768.0), alpha);
}
