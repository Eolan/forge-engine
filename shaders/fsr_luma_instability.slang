// FSR 3.1's luma instability (D-070): AMD's binding numbers of its pass, the bridge, its headers.
// Driven by crates/forge-render/src/fsr.rs.

#define FSR3UPSCALER_BIND_SRV_INPUT_EXPOSURE 0
#define FSR3UPSCALER_BIND_SRV_DILATED_REACTIVE_MASKS 1
#define FSR3UPSCALER_BIND_SRV_DILATED_MOTION_VECTORS 2
#define FSR3UPSCALER_BIND_SRV_FRAME_INFO 3
#define FSR3UPSCALER_BIND_SRV_LUMA_HISTORY 4
#define FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH_MIP1 5
#define FSR3UPSCALER_BIND_SRV_CURRENT_LUMA 6
#define FSR3UPSCALER_BIND_UAV_LUMA_HISTORY 0
#define FSR3UPSCALER_BIND_UAV_LUMA_INSTABILITY 1

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_luma_instability.h"

[shader("compute")]
[numthreads(8, 8, 1)]
void luma_instability_main(int2 iDispatchThreadId : SV_DispatchThreadID)
{
    LumaInstability(iDispatchThreadId);
}
