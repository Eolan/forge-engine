// Impostors drawn (D-075 step 4, #302): far trees as quads facing the camera, each showing its
// tree's baked atlases (crates/forge-render/src/impostor.rs), drawn after the resolve and before
// the sky's compose, over the scene's depth, into the colour and the motion targets. The
// resolve's own entries carry none of it (#349).
import bindless;
import noise;
import sh;
import impostor_layout;

static const float CELL_SIZE = 1024.0;
// The far shadow rays' world (`RAY_SHADOW_FAR` in meshlet.slang, D-075 step 3): the plants'
// opaque shadow proxies in their cards' stead.
static const uint RAY_SHADOW_FAR = 8;

// Must match GpuImpostor in impostor.rs.
struct Impostor
{
    int3 cell;           // its instance's culling sphere's middle, in the scene's cells: where
    uint kind;           // the culls measure its fade from; its atlases and fade
    float3 local;
    float scale;         // its instance's (uniform)
    float4 rotation;     // its instance's (a quaternion)
    float3 middle;       // from the culling sphere's middle to its baked sphere's, metres
    float cull_radius;   // its instance's culling sphere: its fade's crossing
    uint id;             // its instance's id: the same offset of its crossing as the culls'
    float ladder_px;     // the crossing before its meshes' fade (their ladder's), 0 for none
    float ladder_band;
    uint pad;
};

// Must match GpuKind in impostor.rs.
struct Kind
{
    uint albedo;      // sampled atlases, premultiplied by coverage, with mips: the base colour
    uint normal;      // and its coverage; the normal (× 0.5 + 0.5); the share of the sky the
    uint occlusion;   // point sees past the plant itself and the depth in radii (× 0.5 + 0.5)
    float fade_px;    // its shells' fade (`set_fade`): the size on screen and the band
    float fade_band;
    float radius;     // the sphere its frames were baked around
    float reflectance; // its leaves' row's sheen (`GpuMaterial::sheen`): the share reflected head on,
    float specular;   // the highlight's weight and power
    float3 through;   // the share of the sun's light through the leaves (their row's `scatter`)
    float power;
    float3 tone_a;    // the leaves' row's two colours, which each instance mixes by its id: the
    uint pad_a;       // bake drew their mean
    float3 tone_b;
    uint pad_b;
};

// The resolve's per-instance colour hash (`hash_color` in meshlet.slang).
float3 hash_color(uint n)
{
    n = n * 1664525u + 1013904223u;
    n ^= n >> 16;
    n *= 0x7feb352du;
    n ^= n >> 15;
    n *= 0x846ca68bu;
    n ^= n >> 16;
    return float3(n & 0xFF, (n >> 8) & 0xFF, (n >> 16) & 0xFF) / 255.0;
}

// How far the sun's shadow ray's start wanders round the leaves' point, a share of the crown's
// radius (each way).
static const float SHADOW_WANDER = 0.3;

// The light through a leaf (`leaf_transmission` in meshlet.slang).
static const float LEAF_DISTORTION = 0.3;
static const float LEAF_POWER = 4.0;

// Must match GpuImpostorFrame in impostor.rs.
struct Frame
{
    float4x4 view_proj;              // camera-relative → clip, jittered as the scene was drawn
    float4x4 previous_from_current;  // the previous frame's unjittered clip from this one's
    int3 camera_cell;
    uint noise_frame;                // the culls' crossing noise
    float3 camera_local;
    float light_scale;               // the resolve's: sun illuminance × exposure / π
    float3 sun_dir;                  // towards the sun
    float lod_k;                     // p11 × ½ × the render's height / the textures' LOD scale
    float3 sun_color;
    float znear;
    float3 camera_in_scene;          // the camera in the rays' frame
    uint cloud_shadow;               // the clouds' shadow map (sampled, r32f), or 0xFFFFFFFF: none
    float2 jitter;                   // this frame's, pixels
    float2 size;                     // the target's, pixels
    Impostor* instances;
    Kind* kinds;
    float4* sky;                     // the sky's irradiance (sh.slang), or null
    uint64_t tlas;                   // the scene's rays' structure, 0 for none
    float4 cloud_frame;              // the shadow map's first corner (world x, z), 1 / its span
    uint64_t tlas_movers;            // the movers' structure (#79), 0 for none
};

struct Push
{
    Frame* frame;
};

[[vk::push_constant]]
Push push;

struct Varyings
{
    float4 position : SV_Position;
    float3 ray : RAY;                       // the quad's point from the camera (the quad stands at the
                                            // sphere's front, so no written depth is nearer)
    nointerpolation float3 eye : EYE;       // towards the eye, the baked mesh's frame
    nointerpolation float3 centre : CENTRE; // the baked sphere's middle from the camera
    nointerpolation uint instance : INSTANCE;
    nointerpolation float crossing : CROSSING;
};

// The culls' per-instance hash (`hash_unit` in meshlet.slang).
float hash_unit(uint n)
{
    n ^= n >> 16;
    n *= 0x7feb352du;
    n ^= n >> 15;
    n *= 0x846ca68bu;
    n ^= n >> 16;
    return float(n >> 8) * (1.0 / 16777216.0);
}

float3 rotate(float4 q, float3 v)
{
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

// How far a crossing at `size` pixels across, over `band`, is done at `px` (`ladder_crossing`).
float crossed(float px, float size, float band)
{
    band = max(band, 1e-3);
    return saturate((1.0 + band - px / size) / (2.0 * band));
}

static const uint2 CORNERS[6] = { uint2(0, 0), uint2(1, 0), uint2(0, 1), uint2(0, 1), uint2(1, 0), uint2(1, 1) };

[shader("vertex")]
Varyings impostor_vertex_main(uint vertex : SV_VertexID, uint instance : SV_InstanceID)
{
    Frame* f = push.frame;
    Impostor imp = f[0].instances[instance];
    Kind kind = f[0].kinds[imp.kind];
    float3 measured = float3(imp.cell - f[0].camera_cell) * CELL_SIZE + (imp.local - f[0].camera_local);
    float dist = max(length(measured), f[0].znear);
    float3 rel = measured + imp.middle;
    float3 eye = -rel / max(length(rel), 1e-6);
    // Its meshes' fade as the culls measure it (`ladder_crossing`): the impostor shows where they
    // no longer do, once the crossing before it (the cards to the shells) is done.
    float px = 2.0 * imp.cull_radius * f[0].lod_k / dist * (0.85 + 0.3 * hash_unit(imp.id));
    float t = crossed(px, kind.fade_px, kind.fade_band);
    if (imp.ladder_px > 0.0 && crossed(px, imp.ladder_px, imp.ladder_band) < 1.0)
        t = 0.0;
    float3 right;
    float3 up;
    frame_axes(eye, right, up);
    float2 corner = float2(CORNERS[vertex % 6]) * 2.0 - 1.0;
    // The quad at the front of the sphere, facing the camera: every point of the sphere lies
    // behind it, so the depth the pixels write is never nearer than the quad's and the depth test
    // can reject hidden pixels before their shading (`SV_DepthLessEqual`, reversed-Z).
    float radius = kind.radius * imp.scale;
    float3 corner_at = rel + eye * radius + (right * corner.x + up * corner.y) * radius;
    float4 back = float4(-imp.rotation.xyz, imp.rotation.w);
    Varyings o;
    // Not crossed at all, or the camera too near its sphere: off screen, nothing drawn.
    bool drawn = t > 0.0 && length(rel) > 2.0 * radius;
    o.position = drawn ? mul(f[0].view_proj, float4(corner_at, 1.0)) : float4(2.0, 2.0, 2.0, 1.0);
    o.ray = corner_at;
    o.eye = rotate(back, eye);
    o.centre = rel;
    o.instance = instance;
    o.crossing = t;
    return o;
}

// The three frames around the view (`frames_around`): where the quad's point falls in each atlas
// and its gradients (taken before any pixel of the quad is discarded), its weight (none outside
// the frame's picture), the point on the frame's plane and the frame's direction.
struct Views
{
    float2 atlas[3];
    float2 dx[3];
    float2 dy[3];
    float weight[3];
    float3 plane[3];
    float3 direction[3];
};

Views views_of(Kind kind, float3 eye, float3 local)
{
    float2 frames[3];
    float weights[3];
    frames_around(eye, frames, weights);
    Views w;
    for (uint k = 0; k < 3; k++)
    {
        float3 direction = frame_direction(frames[k]);
        float3 right;
        float3 up;
        frame_axes(direction, right, up);
        // The point seen along the frame's own direction: its picture's x right, y down.
        float2 xy = float2(dot(local, right), dot(local, up));
        float2 uv = float2(xy.x, -xy.y) / (2.0 * kind.radius) + 0.5;
        w.weight[k] = all(uv >= 0.0) && all(uv <= 1.0) ? weights[k] : 0.0;
        w.atlas[k] = (frames[k] + clamp(uv, 0.0, 1.0)) / float(FRAMES);
        w.dx[k] = ddx(w.atlas[k]);
        w.dy[k] = ddy(w.atlas[k]);
        w.plane[k] = right * xy.x + up * xy.y;
        w.direction[k] = direction;
    }
    return w;
}

// The frames' base colour and its coverage, blended (premultiplied by the coverage).
float4 blended_albedo(Kind kind, Views w, out float coverage[3])
{
    float4 albedo = float4(0.0);
    for (uint k = 0; k < 3; k++)
    {
        float4 a = sample_image_grad(kind.albedo, SAMPLER_LINEAR_CLAMP, w.atlas[k], w.dx[k], w.dy[k]);
        coverage[k] = a.a;
        albedo += w.weight[k] * a;
    }
    return albedo;
}

// The frames' normal, the point seen (moved along each frame's direction by its depth) and the
// sky it sees, blended and premultiplied by the coverage.
void blend_rest(Kind kind, Views w, float coverage[3], out float3 normal, out float3 point, out float open)
{
    normal = float3(0.0);
    point = float3(0.0);
    open = 0.0;
    for (uint k = 0; k < 3; k++)
    {
        normal += w.weight[k] * sample_image_grad(kind.normal, SAMPLER_LINEAR_CLAMP, w.atlas[k], w.dx[k], w.dy[k]).rgb;
        float2 o = sample_image_grad(kind.occlusion, SAMPLER_LINEAR_CLAMP, w.atlas[k], w.dx[k], w.dy[k]).rg;
        open += w.weight[k] * o.x;
        point += w.weight[k] * (w.plane[k] * coverage[k] + w.direction[k] * ((o.y * 2.0 - 1.0) * kind.radius));
    }
}

// Whether the sun is hidden from `relative` (to the camera): one ray into the far shadows' world
// (D-075 step 3: the plants' opaque proxies, the plant's own among them, every other instance
// opaque), as the far cards' shadow rays meet it, then into the movers' as the resolve's rays do
// (`ray_blocked_except` in meshlet.slang): a plane or a ship passing over shades the far woods as
// it shades the near trees (the owner, 2026-10-25).
bool blocked_in(uint64_t tlas, RayDesc ray)
{
    if (tlas == 0)
        return false;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER | RAY_FLAG_FORCE_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(tlas), RAY_FLAG_NONE, RAY_SHADOW_FAR, ray);
    q.Proceed();
    return q.CommittedStatus() == COMMITTED_TRIANGLE_HIT;
}

bool sun_blocked(Frame* f, float3 relative, float3 sun)
{
    RayDesc ray;
    ray.Origin = f[0].camera_in_scene + relative + sun * 0.02;
    ray.Direction = sun;
    ray.TMin = 0.0;
    ray.TMax = 1e5;
    return blocked_in(f[0].tlas, ray) || blocked_in(f[0].tlas_movers, ray);
}

// The sun's share through the clouds (#145) at `world` (the rays' frame), as the resolve takes it
// (`cloud_light` in meshlet.slang): the shadow map holds it for the ground, so a point `y` up
// reads it `y / sun.y` along the sun's foot. Without it the impostors stood in full sun under a
// cloud's shadow, lime green beside the shells and cards they take over from (the owner,
// 2026-10-25: trees "changing colour very strongly" while flying in). 1 without clouds, or off
// the map.
float cloud_light(Frame* f, float3 world, float3 sun)
{
    if (f[0].cloud_shadow == 0xFFFFFFFFu)
        return 1.0;
    float2 ground = world.xz - sun.xz * (world.y / max(sun.y, 0.05));
    float2 uv = (ground - f[0].cloud_frame.xy) * f[0].cloud_frame.z;
    if (any(uv < 0.0) || any(uv > 1.0))
        return 1.0;
    return sample_depth_level(f[0].cloud_shadow, SAMPLER_LINEAR_CLAMP, uv, 0.0);
}

struct Shaded
{
    float4 color;
    float2 motion;
    float depth;
};

// A pixel of an impostor: kept where the crossing's noise lets it (the complement of the
// shells') and where its coverage passes the frame's noise (thin edges dithered for DLAA). Its
// point is where the bake saw the surface: its depth is written there, and from there it is lit
// as the resolve lights a surface by the sun and the sky, the sun's shadow one ray from the
// point, moved along the view by the frame's noise so that DLAA averages the leaves' depths as
// SIGMA does the far cards' shadows; the camera's motion at that depth (`motion_at` in
// taa.slang).
Shaded shade(Varyings v)
{
    Frame* f = push.frame;
    Impostor imp = f[0].instances[v.instance];
    Kind kind = f[0].kinds[imp.kind];
    uint2 pixel = uint2(v.position.xy);
    float2 noise = spatio_temporal_noise(pixel, f[0].noise_frame);
    // The cheap tests first: no early depth test here, as the pass writes its depth.
    // The point of the sphere's middle plane facing the camera that the pixel's ray crosses, from
    // the middle, in the baked mesh's frame.
    float3 e = normalize(v.centre);
    float3 local = rotate(float4(-imp.rotation.xyz, imp.rotation.w),
        v.ray * (dot(v.centre, e) / dot(v.ray, e)) - v.centre) / imp.scale;
    Views w = views_of(kind, v.eye, local);
    if (noise.x >= v.crossing)
        discard;
    float coverages[3];
    float4 albedo = blended_albedo(kind, w, coverages);
    if (albedo.a <= noise.y)
        discard;
    float3 normal;
    float3 point;
    float open;
    blend_rest(kind, w, coverages, normal, point, open);
    float coverage = max(albedo.a, 1e-4);
    // Its own instance's mix of its leaves' colours over the bake's (their mean), on the leaves
    // alone, told by their colour: the wood is not green (with the leaves' ratio, up to twice
    // their red, a trunk would turn pink).
    float3 baked = albedo.rgb / coverage;
    float3 mean = 0.5 * (kind.tone_a + kind.tone_b);
    float3 tone = lerp(kind.tone_a, kind.tone_b, hash_color(imp.id).x);
    float leafy = saturate(4.0 * (baked.g - max(baked.r, baked.b)) / max(baked.g, 1e-4));
    float3 base = baked * lerp(float3(1.0), tone / max(mean, 1e-4), leafy);
    // The leaves' average normal, as long as they agree: lit by it, the average of their light.
    float3 spread = rotate(imp.rotation, normal / coverage * 2.0 - 1.0);
    float agree = min(length(spread), 1.0);
    float3 n = agree > 1e-4 ? spread / agree : float3(0.0, 1.0, 0.0);
    float3 seen = point / coverage;
    float3 relative = v.centre + rotate(imp.rotation, seen * imp.scale);
    float3 sun = normalize(f[0].sun_dir);
    float3 view = -normalize(relative);
    float ndl = dot(n, sun) * agree;
    // How much of the sun reaches the point, its own crown's proxy and the world's in the way.
    float shadow = 0.0;
    if (sun.y > 0.0)
    {
        // The ray's start wanders over a ball a third of the crown's radius across, by the
        // frame's noise: DLAA's history averages the frames into a soft shadow, as SIGMA blurs
        // the far cards' (one fixed ray lit or shaded each leaf outright: twice their contrast).
        float3 wander = float3(
            interleaved_gradient_noise(pixel, f[0].noise_frame, float2(23.0, 11.0)),
            interleaved_gradient_noise(pixel, f[0].noise_frame, float2(5.0, 37.0)),
            interleaved_gradient_noise(pixel, f[0].noise_frame, float2(41.0, 3.0))) - 0.5;
        shadow = sun_blocked(f, relative + wander * (SHADOW_WANDER * kind.radius * imp.scale), sun) ? 0.0 : 1.0;
        shadow *= cloud_light(f, f[0].camera_in_scene + relative, sun);
    }
    // The sky's light on leaves that agree that much: from their normal's, towards its mean.
    float3 sky = f[0].sky != nullptr
        ? lerp(f[0].sky[0].xyz * 0.282095, sh_irradiance(f[0].sky, n), agree)
        : float3(0.03);
    // As `sun_light` lights a rough row: the diffuse part less what it reflects, the sun's
    // highlight, and the sky's light reflected (`sky_reflection`: a row this rough reflects the
    // irradiance), with the light through the leaves (`leaf_transmission`).
    // The sky's reflection dimmed by the share of the sky the point sees past the plant (the
    // reflection's occlusion in `sky_reflection`): undimmed, it paled the olives. Its diffuse
    // light is not, as the shells' and the far cards' are not (the screen's occlusion is weak that
    // far): dimmed, the crowns in shade read near black beside their shells.
    float ao = saturate(open / coverage);
    float roughness = sqrt(sqrt(2.0 / (kind.power + 2.0)));
    float f0 = kind.reflectance;
    float nv = saturate(dot(n, view));
    float fresnel = f0 + (max(1.0 - roughness, f0) - f0) * pow(1.0 - nv, 5.0);
    float reflected = fresnel * saturate(pow(nv + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao);
    float highlight = ndl > 0.0 ? pow(max(dot(n, normalize(sun + view)), 0.0), kind.power) * kind.specular : 0.0;
    float3 l = normalize(sun + n * LEAF_DISTORTION);
    float lobe = pow(saturate(dot(view, -l)), LEAF_POWER);
    float3 sun_light = f[0].sun_color * shadow;
    float3 color = base * (max(ndl, 0.0) * sun_light + sky) * (1.0 - fresnel)
        + highlight * sun_light + reflected * sky + base * kind.through * sun_light * lobe;
    Shaded o;
    o.color = float4(color * f[0].light_scale, 1.0);
    float4 clip = mul(f[0].view_proj, float4(relative, 1.0));
    // Never nearer than the quad (the blend of three frames' points can stray past the sphere).
    o.depth = min(clip.z / max(clip.w, 1e-6), v.position.z);
    float2 uv = (v.position.xy - f[0].jitter) / f[0].size;
    float4 previous = mul(f[0].previous_from_current, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, o.depth, 1.0));
    o.motion = previous.w > 1e-9
        ? float2(previous.x / previous.w * 0.5 + 0.5, 0.5 - previous.y / previous.w * 0.5) - uv
        : float2(4.0, 4.0);
    return o;
}

struct Out
{
    float4 color : SV_Target0;
    float2 motion : SV_Target1;
    float depth : SV_DepthLessEqual;
};

[shader("fragment")]
Out impostor_fragment_main(Varyings v)
{
    Shaded s = shade(v);
    Out o;
    o.color = s.color;
    o.motion = s.motion;
    o.depth = s.depth;
    return o;
}

struct ColorOut
{
    float4 color : SV_Target0;
    float depth : SV_DepthLessEqual;
};

// Without the motion target: the motion vectors pass, after this one, takes the impostors'
// motion from their depth as everything else's.
[shader("fragment")]
ColorOut impostor_color_main(Varyings v)
{
    Shaded s = shade(v);
    ColorOut o;
    o.color = s.color;
    o.depth = s.depth;
    return o;
}
