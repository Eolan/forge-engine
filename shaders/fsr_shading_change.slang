// FSR 3.1's shading change (D-070): AMD's binding numbers of its pass, the bridge, its headers.
// Driven by crates/forge-render/src/fsr.rs.

#define FSR3UPSCALER_BIND_SRV_SPD_MIPS 0
#define FSR3UPSCALER_BIND_UAV_SHADING_CHANGE 0

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_shading_change.h"

[shader("compute")]
[numthreads(8, 8, 1)]
void shading_change_main(int2 iDispatchThreadId : SV_DispatchThreadID)
{
    ShadingChange(iDispatchThreadId);
}
