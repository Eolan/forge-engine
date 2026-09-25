// The ACES 2.0 check (issue #76, crates/forge-render/src/tonecheck.rs): both GPU paths of
// ACES 2.0 over a fixed set of scene colours, for the CPU to compare with its own transform.
import tonemap;

struct CheckPush
{
    float4* input;  // xyz: linear Rec.709 scene colours
    float4* output; // per colour: the table's display colour, then the per-pixel one
    uint count;
    uint pad;
    ToneTables tables;
};

[[vk::push_constant]]
CheckPush check_push;

[shader("compute")]
[numthreads(64, 1, 1)]
void check_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x >= check_push.count)
        return;
    float3 colour = check_push.input[id.x].xyz;
    check_push.output[2 * id.x] = float4(tonemap(colour, TONEMAP_ACES2, check_push.tables), 1.0);
    check_push.output[2 * id.x + 1] = float4(tonemap(colour, TONEMAP_ACES2_ANALYTIC, check_push.tables), 1.0);
}
