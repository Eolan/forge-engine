// The ACES 2.0 check (issues #76, #94; crates/forge-render/src/tonecheck.rs): both GPU paths of
// ACES 2.0, SDR and through the 1000-nit HDR preset, over a fixed set of scene colours, for the
// CPU to compare with its own transform.
import aces2;
import tonemap;

struct CheckPush
{
    float4* input;  // xyz: linear Rec.709 scene colours
    float4* output; // per colour: SDR table, SDR per pixel, HDR table, HDR per pixel (PQ)
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
    ToneTables tables = check_push.tables;
    check_push.output[4 * id.x] = float4(tonemap(colour, TONEMAP_ACES2, tables), 1.0);
    check_push.output[4 * id.x + 1] = float4(tonemap(colour, TONEMAP_ACES2_ANALYTIC, tables), 1.0);
    check_push.output[4 * id.x + 2] = float4(aces2_hdr_lut(colour, tables.hdr_lut, tables.hdr_max), 1.0);
    check_push.output[4 * id.x + 3] = float4(pq_encode(aces2_analytic_hdr(colour, tables.hdr) * 100.0), 1.0);
}
