// Supersampling's last pass (D-045: SSAA 2 × 2 for screenshots), driven by
// crates/forge-app/src/ssaa.rs: the frame drawn at twice the window's width and height, each
// window pixel the mean of its four. An sRGB image is read as linear light and written back
// encoded, so the mean is light's.
import bindless;

struct SsaaPush
{
    uint image;
};

[[vk::push_constant]]
SsaaPush ssaa_push;

struct Varyings
{
    float4 position : SV_Position;
};

[shader("vertex")]
Varyings vert_main(uint id : SV_VertexID)
{
    Varyings o;
    o.position = float4((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0, 0.0, 1.0);
    return o;
}

[shader("fragment")]
float4 frag_main(Varyings input) : SV_Target
{
    int2 p = int2(input.position.xy) * 2;
    float4 sum = 0.0;
    for (int y = 0; y < 2; y++)
        for (int x = 0; x < 2; x++)
            sum += g_sampled_images[NonUniformResourceIndex(ssaa_push.image)].Load(int3(p + int2(x, y), 0));
    return sum * 0.25;
}
