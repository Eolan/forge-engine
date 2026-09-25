// The loading screen (issue #25): a ring of twelve dots turning while a demo prepares its
// scene on another thread. Drawn into the swapchain image by crates/forge-app/src/loading.rs.

struct Push
{
    float2 size; // the target, pixels
    float time;  // seconds since the loading started
    float pad;
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
static const uint DOTS = 12;

[shader("fragment")]
float4 frag_main(VertexOut input) : SV_Target
{
    float scale = min(push.size.x, push.size.y);
    float2 p = (input.position.xy - 0.5 * push.size) / scale; // centred, y down
    float3 color = float3(0.012, 0.014, 0.020);
    float lead = frac(push.time * 0.9); // turns per second
    for (uint k = 0; k < DOTS; k++)
    {
        float a = float(k) / float(DOTS) * 2.0 * PI;
        float2 centre = 0.06 * float2(sin(a), -cos(a));
        // The leading dot is the brightest; the others fade behind it, clockwise.
        float age = frac(lead - float(k) / float(DOTS));
        float glow = pow(1.0 - age, 3.0);
        float disc = saturate((0.008 - length(p - centre)) * scale + 0.5); // antialiased over a pixel
        color += disc * lerp(0.08, 1.0, glow) * float3(0.55, 0.70, 0.90);
    }
    return float4(color, 1.0);
}
