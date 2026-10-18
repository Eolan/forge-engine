// FSR 3.1's sharpening, RCAS (D-070): AMD's binding numbers of its pass, the bridge, its headers.
// Driven by crates/forge-render/src/fsr.rs.

#define FSR3UPSCALER_BIND_SRV_INPUT_EXPOSURE 0
#define FSR3UPSCALER_BIND_SRV_RCAS_INPUT 1
#define FSR3UPSCALER_BIND_UAV_UPSCALED_OUTPUT 0

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_rcas.h"

[shader("compute")]
[numthreads(64, 1, 1)]
void rcas_main(uint3 LocalThreadId : SV_GroupThreadID, uint3 WorkGroupId : SV_GroupID, uint3 Dtid : SV_DispatchThreadID)
{
    RCAS(LocalThreadId, WorkGroupId, Dtid);
}
