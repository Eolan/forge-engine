// The surface model's shared terms (D-086): GGX (Trowbridge-Reitz) with the height-correlated
// Smith visibility and Schlick's Fresnel, as Falcor's and every current engine's, and its
// directional albedo over a sky by the split sum. The resolve (meshlet.slang) and the impostors
// (impostor_draw.slang) light by the same, so a crown's cards and its impostor stay one colour.
// A row's lobe is still given as a Blinn-Phong power (RenderLayer::specular_power): GGX's α² is
// 2 / (power + 2).
module brdf;

static const float GGX_PI = 3.14159265;

// The default highlight weight of a row (RenderLayer's `specular`): under GGX a row's weight only
// dims its highlight (the paths' benches at 0.02, damp prints at 0.03); weights over it made up
// for Blinn-Phong's unnormalised lobe, which GGX's normalisation now gives.
public static const float DEFAULT_SPECULAR = 0.05;

// The share of the GGX highlight a row with highlight weight `weight` keeps.
public float highlight_share(float weight)
{
    return saturate(weight / DEFAULT_SPECULAR);
}

// The light a GGX surface (lobe α², reflectance at normal incidence `f0`) facing `n` sends towards
// the unit `view` per unit of illuminance from the unit direction `light` (at normal incidence),
// in the resolve's units: π times the BRDF times n·l.
public float3 ggx_light(float3 n, float3 view, float3 light, float a2, float3 f0)
{
    float nl = saturate(dot(n, light));
    if (nl <= 0.0)
        return float3(0.0);
    float nv = max(dot(n, view), 1e-4);
    float3 h = normalize(light + view);
    float nh = saturate(dot(n, h));
    float vh = saturate(dot(view, h));
    a2 = max(a2, 1e-7);
    float d = nh * nh * (a2 - 1.0) + 1.0;
    float distribution = a2 / (GGX_PI * d * d);
    float visibility = 0.5 / max(nl * sqrt(nv * nv * (1.0 - a2) + a2) + nv * sqrt(nl * nl * (1.0 - a2) + a2), 1e-7);
    float3 fresnel = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
    return GGX_PI * distribution * visibility * fresnel * nl;
}

// GGX's directional albedo over a sky (D-086): Karis's split sum, its DFG term by Lazarov's
// analytic fit (Karis, "Physically Based Shading on Mobile", 2014) of `roughness` (perceptual) and
// n·v, as F0 times A plus B, B gated off under an F0 of 0.02 as Unreal's (the fit's B dips under
// zero when rough); with `multiscatter`, Fdez-Agüera's compensation (JCGT 2019) of the light
// single-scattering GGX loses between its microfacets, which grows with the roughness: a rough
// metal keeps its light, as a real one does. The share the reflection takes, and the diffuse
// loses. Replaced Lagarde's roughness-bounded Schlick, which kept a rough metal at its F0 (as
// bright as white paper) and a rough dielectric's grazing rise too high.
public float3 environment_specular(float3 f0, float roughness, float nv, bool multiscatter)
{
    const float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
    const float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
    float4 q = roughness * c0 + c1;
    float a004 = min(q.x * q.x, exp2(-9.28 * nv)) * q.x + q.y;
    float2 ab = float2(-1.04, 1.04) * a004 + q.zw;
    float3 single = max(f0 * ab.x + ab.y * saturate(50.0 * f0.g), 0.0);
    if (!multiscatter)
        return single;
    float ess = ab.x + ab.y;
    float3 average = f0 + (1.0 - f0) / 21.0;
    float3 multiple = single * average / (1.0 - (1.0 - ess) * average);
    return single + multiple * (1.0 - ess);
}
